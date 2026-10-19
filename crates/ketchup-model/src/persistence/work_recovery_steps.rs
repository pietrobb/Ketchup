//! Idea #49: an edit of a saved document appends one step to its work-recovery
//! checkpoint instead of writing the whole document and Undo history again.
//!
//! A checkpoint is the whole native file written at the first edit (its base),
//! then one step per later edit. A step names what the edit changed in the Undo
//! history: how many revisions at its start were dropped, how many after them
//! stay, the revisions added after those, the Undo position and the next
//! revision id. An added revision's snapshot is named as chunks of the revision
//! before it plus the chunks the step stores. Each step is a native container
//! of one entry, so it carries that format's entry checksum: a step cut short
//! by a crash is told apart from the steps before it. Opening assembles the base
//! and its steps into one native file and opens that like any other.
//!
//! The base is written again whole when the steps cannot name the change (the
//! history no longer shares a revision with the checkpoint, blobs or extensions
//! changed, a step does not read) or once the steps outweigh the base.

use std::borrow::Cow;

use sha2::{Digest, Sha256};

use super::work_recovery::read_observed_work_recovery;
use super::*;

/// The one entry of a step's container.
const STEP_ENTRY: &str = "step.bin";
/// Set in a chunk number that names a chunk the step stores; others name a
/// chunk of the revision before.
const STEP_CHUNK_INLINE: u32 = 1 << 31;
const STEP_PROGRAM_NONE: u8 = 0;
const STEP_PROGRAM_INLINE: u8 = 1;
const STEP_PROGRAM_AS_PREVIOUS: u8 = 2;
/// Room the assembled file needs beyond the checkpoint's bytes: entry framing
/// and the chunk table's entries for `document.bin`.
const ASSEMBLY_MARGIN: usize = 1024 * 1024;

/// A count or index the file names as `u32`, as the `usize` it indexes with.
fn index(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// A length or count as the `u64` the budgets count in.
fn wide(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Appends the step that brings the checkpoint of `path`, observed as
/// `expected`, to `document`. `None` when the checkpoint must be written whole.
/// The same four identity checks guard the append as a whole write.
pub(super) fn append_work_recovery_step(
    path: &Path,
    document: &DocumentStore,
    container: &ContainerData,
    base_identity: FileIdentity,
    expected: FileIdentity,
) -> Result<Option<FileIdentity>, FilePersistenceError> {
    let _lock = lock_saves(path)?;
    if read_native_document_identity(path)? != base_identity {
        return Err(FilePersistenceError::ExternalConflict);
    }
    let Some(observed) = read_observed_work_recovery(path, Some(expected))? else {
        return Err(FilePersistenceError::ExternalConflict);
    };
    let Some(step) = plan_step(&observed.bytes, document, container, base_identity) else {
        return Ok(None);
    };
    if read_native_document_identity(path)? != base_identity {
        return Err(FilePersistenceError::ExternalConflict);
    }
    check_work_recovery_identity(path, Some(expected))?;
    let mut framed = Vec::with_capacity(8 + step.len());
    push_u64(
        &mut framed,
        u64_len(step.len()).map_err(FilePersistenceError::Format)?,
    );
    framed.extend_from_slice(&step);
    let recovery = work_recovery_path(path);
    let observed_len = wide(observed.bytes.len());
    let mut file = fs::OpenOptions::new().write(true).open(&recovery)?;
    let appended = std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(observed_len))
        .and_then(|_| file.write_all(&framed))
        .and_then(|()| file.sync_all());
    if let Err(error) = appended {
        // A step cut short would hide every later one: the checkpoint stays as
        // this writer observed it, so the edit can be retried.
        let _ = file.set_len(observed_len).and_then(|()| file.sync_all());
        return Err(FilePersistenceError::Io(error));
    }
    let mut hasher = observed.hasher;
    hasher.update(&framed);
    let identity = FileIdentity {
        byte_len: wide(observed.bytes.len() + framed.len()),
        sha256: hasher.finalize().into(),
    };
    let parent = recovery
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    sync_parent_directory(parent).map_err(|error| FilePersistenceError::Published {
        path: recovery.clone(),
        identity,
        error,
    })?;
    Ok(Some(identity))
}

/// The base and the steps of a checkpoint made from `base_identity`, or `None`
/// for a checkpoint of another format or file.
fn split_checkpoint(bytes: &[u8], base_identity: FileIdentity) -> Option<(&[u8], &[u8])> {
    let mut reader = Reader::new(bytes);
    if reader.take(WORK_RECOVERY_MAGIC.len()).ok()? != WORK_RECOVERY_MAGIC
        || reader.u16().ok()? != WORK_RECOVERY_SCHEMA
    {
        return None;
    }
    let byte_len = reader.u64().ok()?;
    let sha256: [u8; 32] = reader.take(32).ok()?.try_into().ok()?;
    if (FileIdentity { byte_len, sha256 }) != base_identity {
        return None;
    }
    let payload_len = usize::try_from(reader.u64().ok()?).ok()?;
    let payload = reader.take(payload_len).ok()?;
    Some((payload, &bytes[reader.cursor..]))
}

/// The base of a checkpoint: its container and the Undo history in it.
struct Base<'a> {
    entries: Vec<ContainerEntry<'a>>,
    document_checksum: [u8; 32],
    history: ParsedHistory<'a>,
}

/// `verify` checks every entry's checksum; the writer reads bytes whose file
/// identity it already checked.
fn read_base(payload: &[u8], verify: bool) -> Result<Base<'_>, PersistenceError> {
    let entries = container_directory(payload)?;
    if verify
        && let Some(entry) = entries
            .iter()
            .find(|entry| crate::graph::sha256_bytes(entry.content) != entry.checksum)
    {
        return Err(PersistenceError::ContainerChecksumMismatch(
            entry.path.clone(),
        ));
    }
    let entry = |path: &str| {
        entries
            .iter()
            .find(|entry| entry.path == path && entry.required)
    };
    let document = entry("document.bin").ok_or(PersistenceError::MissingDocumentEntry)?;
    let (document, document_checksum) = (document.content, document.checksum);
    let history = entry("history.bin").ok_or(PersistenceError::InvalidRevisionHistory)?;
    let history = read_history_records(history.content, document, HISTORY_BUDGET)?;
    if history.schema < 4 {
        return Err(PersistenceError::InvalidRevisionHistory);
    }
    Ok(Base {
        entries,
        document_checksum,
        history,
    })
}

/// The steps in `bytes` whose framing and checksum hold, then why the first one
/// that does not was left out with every step after it.
fn read_steps(bytes: &[u8]) -> (Vec<Vec<u8>>, Option<PersistenceError>) {
    let mut reader = Reader::new(bytes);
    let mut steps = Vec::new();
    while !reader.is_finished() {
        let step = reader
            .u64()
            .and_then(|length| {
                usize::try_from(length)
                    .map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)
            })
            .and_then(|length| reader.take(length))
            .and_then(|container| {
                let mut entries = container_entries(container)?;
                match entries.remove(STEP_ENTRY) {
                    Some((true, content)) if entries.is_empty() => Ok(content),
                    _ => Err(PersistenceError::InvalidRevisionHistory),
                }
            });
        match step {
            Ok(step) => steps.push(step),
            Err(error) => return (steps, Some(error)),
        }
    }
    (steps, None)
}

struct Step<'a> {
    skip: usize,
    keep: usize,
    cursor: usize,
    next_revision_id: u64,
    chunks: Vec<&'a [u8]>,
    revisions: Vec<StepRevision<'a>>,
}

struct StepRevision<'a> {
    id: u64,
    batch_digest: String,
    origin: RevisionOrigin,
    checkpoint: Option<String>,
    program: StepProgram<'a>,
    length: usize,
    sha256: [u8; 32],
    numbers: &'a [u8],
}

enum StepProgram<'a> {
    None,
    Inline(&'a [u8]),
    AsPrevious,
}

fn parse_step(bytes: &[u8]) -> Result<Step<'_>, PersistenceError> {
    let mut reader = Reader::new(bytes);
    let skip = index(reader.u32()?);
    let keep = index(reader.u32()?);
    let cursor = index(reader.u32()?);
    let next_revision_id = reader.u64()?;
    let chunk_count = reader.count_with_limit(MAX_COLLECTION_ITEMS)?;
    let mut chunks = Vec::with_capacity(index(chunk_count));
    for _ in 0..chunk_count {
        let length = index(reader.u32()?);
        chunks.push(reader.take(length)?);
    }
    let count = reader.count_with_limit(FILE_HISTORY_REVISIONS)?;
    let mut revisions = Vec::with_capacity(index(count));
    for _ in 0..count {
        let id = reader.u64()?;
        let batch_digest = reader.string()?;
        let origin = read_revision_origin(&mut reader)?;
        let checkpoint = match reader.u8()? {
            0 => None,
            1 => Some(reader.string()?),
            _ => return Err(PersistenceError::InvalidRevisionHistory),
        };
        let program = match reader.u8()? {
            STEP_PROGRAM_NONE => StepProgram::None,
            STEP_PROGRAM_INLINE => {
                let length = index(reader.u32()?);
                if length > crate::document::MAX_RULE_PROGRAM_BYTES {
                    return Err(PersistenceError::ResourceLimit);
                }
                StepProgram::Inline(reader.take(length)?)
            }
            STEP_PROGRAM_AS_PREVIOUS => StepProgram::AsPrevious,
            _ => return Err(PersistenceError::InvalidRevisionHistory),
        };
        let (length, sha256, numbers) = read_chunk_list(&mut reader, MAX_SNAPSHOT_BYTES)?;
        revisions.push(StepRevision {
            id,
            batch_digest,
            origin,
            checkpoint,
            program,
            length,
            sha256,
            numbers,
        });
    }
    if !reader.is_finished() {
        return Err(PersistenceError::TrailingBytes);
    }
    Ok(Step {
        skip,
        keep,
        cursor,
        next_revision_id,
        chunks,
        revisions,
    })
}

/// An Undo history as a checkpoint's base and steps hold it.
struct History<T> {
    revisions: Vec<T>,
    cursor: usize,
    next_revision_id: u64,
}

impl<T> History<T> {
    /// `step` applied; `added` makes each revision it adds from the history so far.
    fn apply<'a>(
        &mut self,
        step: &Step<'a>,
        mut added: impl FnMut(&[T], &Step<'a>, &StepRevision<'a>) -> Result<T, PersistenceError>,
    ) -> Result<(), PersistenceError> {
        if step.keep == 0 || step.skip + step.keep > self.revisions.len() {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        self.revisions.drain(..step.skip);
        self.revisions.truncate(step.keep);
        for revision in &step.revisions {
            let revision = added(&self.revisions, step, revision)?;
            self.revisions.push(revision);
        }
        if step.cursor >= self.revisions.len() {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        self.cursor = step.cursor;
        self.next_revision_id = step.next_revision_id;
        Ok(())
    }
}

/// What the writer compares of a revision the checkpoint holds. A revision keeps
/// its program once written: a program is bound only to a revision just made.
#[derive(PartialEq, Eq)]
struct HeldRevision {
    id: u64,
    batch_digest: String,
    origin: RevisionOrigin,
    checkpoint: Option<String>,
    length: usize,
    sha256: [u8; 32],
}

impl HeldRevision {
    fn of(revision: &Revision, encoded: &crate::document::EncodedSnapshot) -> Self {
        Self {
            id: revision.id(),
            batch_digest: revision.batch_digest().to_owned(),
            origin: revision.origin(),
            checkpoint: revision.checkpoint().map(str::to_owned),
            length: encoded.bytes.len(),
            sha256: encoded.sha256,
        }
    }
}

/// The step that brings the checkpoint `observed` to `document`, or `None` when
/// only a whole checkpoint can.
fn plan_step(
    observed: &[u8],
    document: &DocumentStore,
    container: &ContainerData,
    base_identity: FileIdentity,
) -> Option<Vec<u8>> {
    let (payload, steps) = split_checkpoint(observed, base_identity)?;
    let base = read_base(payload, false).ok()?;
    let (steps, damaged) = read_steps(steps);
    if damaged.is_some() {
        return None;
    }
    let mut held = History {
        revisions: base
            .history
            .records
            .iter()
            .map(|record| {
                let (length, sha256) = match record.encoded_snapshot {
                    HistorySnapshot::Document(bytes) => (bytes.len(), base.document_checksum),
                    HistorySnapshot::Whole { bytes, checksum } => (bytes.len(), checksum),
                    HistorySnapshot::Chunks {
                        length, checksum, ..
                    } => (length, checksum),
                };
                HeldRevision {
                    id: record.revision_id,
                    batch_digest: record.batch_digest.clone(),
                    origin: record.origin,
                    checkpoint: record.checkpoint.clone(),
                    length,
                    sha256,
                }
            })
            .collect(),
        cursor: base.history.cursor,
        next_revision_id: base.history.next_revision_id,
    };
    for step in &steps {
        held.apply(&parse_step(step).ok()?, |_, _, revision| {
            Ok(HeldRevision {
                id: revision.id,
                batch_digest: revision.batch_digest.clone(),
                origin: revision.origin,
                checkpoint: revision.checkpoint.clone(),
                length: revision.length,
                sha256: revision.sha256,
            })
        })
        .ok()?;
    }

    let revisions = document.revision_history().collect::<Vec<_>>();
    let encoded = revisions
        .iter()
        .map(|revision| encoded_snapshot(revision.snapshot()))
        .collect::<Vec<_>>();
    let current = revisions
        .iter()
        .zip(&encoded)
        .map(|(revision, encoded)| HeldRevision::of(revision, encoded))
        .collect::<Vec<_>>();
    let skip = held
        .revisions
        .iter()
        .position(|revision| *revision == current[0])?;
    let keep = held.revisions[skip..]
        .iter()
        .zip(&current)
        .take_while(|(held, current)| held == current)
        .count();
    if !same_side_entries(&base.entries, container, &revisions) || !fits(&revisions, &encoded) {
        return None;
    }
    let step = encode_step(skip, keep, document, &revisions, &encoded).ok()?;
    let step = encode_container(BTreeMap::from([(STEP_ENTRY.to_owned(), (true, step))])).ok()?;
    let steps_len = observed.len() - payload.len() + 8 + step.len();
    let assembled = observed.len() + 8 + step.len() + ASSEMBLY_MARGIN;
    (steps_len <= payload.len()
        && assembled <= MAX_SIDECAR_BYTES
        && assembled + encoded[document.history_cursor()].bytes.len() <= MAX_CONTAINER_BYTES)
        .then_some(step)
}

/// Whether the blobs and extensions a whole checkpoint of `revisions` would hold
/// are those of the base.
fn same_side_entries(
    entries: &[ContainerEntry<'_>],
    container: &ContainerData,
    revisions: &[&Revision],
) -> bool {
    let mut imported = BTreeSet::new();
    for revision in revisions {
        imported.extend(imported_source_blob_hashes(revision.snapshot()));
    }
    if imported
        .iter()
        .any(|hash| !container.blobs.contains_key(hash))
    {
        return false;
    }
    // A blob is named by its content's hash, so its name stands for it.
    let mut expected = BTreeMap::<String, (bool, Option<&[u8]>)>::new();
    for hash in container.blobs.keys() {
        if !container.imported_source_blobs.contains(hash) || imported.contains(hash) {
            expected.insert(format!("blobs/{hash}"), (false, None));
        }
    }
    for extension in container.extensions.values() {
        expected.insert(
            format!("extensions/{}/{}", extension.namespace, extension.path),
            (extension.required, Some(extension.bytes.as_slice())),
        );
    }
    let mut held = 0;
    for entry in entries {
        if matches!(
            entry.path.as_str(),
            "document.bin" | "history.bin" | PROGRAM_ENTRY
        ) {
            continue;
        }
        held += 1;
        let matches = expected.get(&entry.path).is_some_and(|(required, bytes)| {
            *required == entry.required && bytes.is_none_or(|bytes| bytes == entry.content)
        });
        if !matches {
            return false;
        }
    }
    held == expected.len()
}

/// Whether the history the steps assemble stays within the budget a whole
/// checkpoint of it is read with.
fn fits(revisions: &[&Revision], encoded: &[Arc<crate::document::EncodedSnapshot>]) -> bool {
    let mut snapshots = 0u64;
    let mut items = wide(revisions.len());
    for (revision, encoded) in revisions.iter().zip(encoded) {
        if encoded.bytes.len() > HISTORY_BUDGET.snapshot_bytes {
            return false;
        }
        snapshots += wide(encoded.bytes.len());
        let chunks = encoded
            .chunks
            .get_or_init(|| history_chunks::chunks(&encoded.bytes))
            .len();
        // A chunk list, the table entries it may add, a program and the strings.
        items += 2 * (wide(chunks) + 1)
            + wide(revision.batch_digest().len())
            + wide(revision.checkpoint().map_or(0, str::len));
    }
    snapshots <= HISTORY_BUDGET.snapshots && items <= u64::from(HISTORY_BUDGET.items)
}

fn encode_step(
    skip: usize,
    keep: usize,
    document: &DocumentStore,
    revisions: &[&Revision],
    encoded: &[Arc<crate::document::EncodedSnapshot>],
) -> Result<Vec<u8>, PersistenceError> {
    let chunks_of = |index: usize| {
        encoded[index]
            .chunks
            .get_or_init(|| history_chunks::chunks(&encoded[index].bytes))
    };
    let mut stored = Vec::new();
    let mut stored_numbers = BTreeMap::<[u8; 32], u32>::new();
    let mut records = Vec::new();
    for index in keep..revisions.len() {
        let (revision, previous) = (revisions[index], revisions[index - 1]);
        write_revision_header(&mut records, revision)?;
        match revision.rule_program() {
            None => push_u8(&mut records, STEP_PROGRAM_NONE),
            Some(program) if previous.rule_program() == Some(program) => {
                push_u8(&mut records, STEP_PROGRAM_AS_PREVIOUS);
            }
            Some(_) => {
                let program = encoded_rule_program(revision)?.unwrap_or_default();
                push_u8(&mut records, STEP_PROGRAM_INLINE);
                push_u32(&mut records, u32_count(program.len())?);
                records.extend_from_slice(&program);
            }
        }
        let mut before = BTreeMap::new();
        for (number, chunk) in chunks_of(index - 1).iter().enumerate().rev() {
            before.insert(chunk.sha256, u32_count(number)?);
        }
        let snapshot = &encoded[index];
        let chunks = chunks_of(index);
        push_u64(&mut records, u64_len(snapshot.bytes.len())?);
        records.extend_from_slice(&snapshot.sha256);
        push_u32(&mut records, u32_count(chunks.len())?);
        for chunk in chunks {
            let number = match before.get(&chunk.sha256) {
                Some(number) => *number,
                None => {
                    if let Some(number) = stored_numbers.get(&chunk.sha256) {
                        *number
                    } else {
                        let number = STEP_CHUNK_INLINE | u32_count(stored_numbers.len())?;
                        let bytes = &snapshot.bytes[chunk.range.clone()];
                        push_u32(&mut stored, u32_count(bytes.len())?);
                        stored.extend_from_slice(bytes);
                        stored_numbers.insert(chunk.sha256, number);
                        number
                    }
                }
            };
            push_u32(&mut records, number);
        }
    }
    let mut step = Vec::with_capacity(28 + stored.len() + records.len());
    push_u32(&mut step, u32_count(skip)?);
    push_u32(&mut step, u32_count(keep)?);
    push_u32(&mut step, u32_count(document.history_cursor())?);
    push_u64(&mut step, document.next_revision_id());
    push_u32(&mut step, u32_count(stored_numbers.len())?);
    step.extend_from_slice(&stored);
    push_u32(&mut step, u32_count(revisions.len() - keep)?);
    step.extend_from_slice(&records);
    Ok(step)
}

/// A revision as the opened checkpoint assembles it.
struct AssembledRevision<'a> {
    id: u64,
    batch_digest: String,
    origin: RevisionOrigin,
    checkpoint: Option<String>,
    program: Option<Cow<'a, [u8]>>,
    length: usize,
    sha256: [u8; 32],
    chunks: Vec<&'a [u8]>,
}

/// The native file the base `payload` and the steps in `steps` assemble to,
/// and why a step was left out with every one after it.
pub(super) fn assemble_checkpoint(
    payload: &[u8],
    steps: &[u8],
) -> Result<(Vec<u8>, Option<String>), PersistenceError> {
    let (steps, damaged) = read_steps(steps);
    let mut damaged = damaged.map(|error| (steps.len(), error));
    if steps.is_empty() {
        return Ok((payload.to_vec(), damaged.map(damage_reason)));
    }
    let base = read_base(payload, true)?;
    let mut history = History {
        revisions: Vec::with_capacity(base.history.records.len()),
        cursor: base.history.cursor,
        next_revision_id: base.history.next_revision_id,
    };
    for record in &base.history.records {
        let (length, sha256, chunks) = match record.encoded_snapshot {
            HistorySnapshot::Document(bytes) => (
                bytes.len(),
                base.document_checksum,
                history_chunks::chunks(bytes)
                    .into_iter()
                    .map(|chunk| &bytes[chunk.range])
                    .collect(),
            ),
            HistorySnapshot::Whole { bytes, checksum } => (bytes.len(), checksum, vec![bytes]),
            HistorySnapshot::Chunks {
                length,
                checksum,
                numbers,
            } => (
                length,
                checksum,
                numbers
                    .chunks_exact(4)
                    .map(|number| {
                        let number = u32::from_le_bytes(number.try_into().expect("four bytes"));
                        base.history
                            .chunks
                            .get(index(number))
                            .copied()
                            .ok_or(PersistenceError::InvalidRevisionHistory)
                    })
                    .collect::<Result<_, _>>()?,
            ),
        };
        let program = match record.rule_program {
            None => None,
            Some(HistoryProgram::Inline(bytes)) => Some(Cow::Borrowed(bytes)),
            Some(HistoryProgram::Chunks {
                length,
                checksum,
                numbers,
            }) => Some(
                HistorySnapshot::Chunks {
                    length,
                    checksum,
                    numbers,
                }
                .assemble(&base.history.chunks)?,
            ),
        };
        history.revisions.push(AssembledRevision {
            id: record.revision_id,
            batch_digest: record.batch_digest.clone(),
            origin: record.origin,
            checkpoint: record.checkpoint.clone(),
            program,
            length,
            sha256,
            chunks,
        });
    }
    for (index, step) in steps.iter().enumerate() {
        let applied = parse_step(step).and_then(|step| {
            let mut next = History {
                revisions: history
                    .revisions
                    .iter()
                    .map(AssembledRevision::share)
                    .collect(),
                cursor: history.cursor,
                next_revision_id: history.next_revision_id,
            };
            next.apply(&step, assemble_revision)?;
            Ok(next)
        });
        match applied {
            Ok(next) => history = next,
            Err(error) => {
                damaged = Some((index, error));
                break;
            }
        }
    }
    let file = assemble_file(&base.entries, &history)?;
    Ok((file, damaged.map(damage_reason)))
}

fn damage_reason((index, error): (usize, PersistenceError)) -> String {
    format!("step {} of the checkpoint is damaged: {error}", index + 1)
}

impl<'a> AssembledRevision<'a> {
    fn share(&self) -> Self {
        Self {
            id: self.id,
            batch_digest: self.batch_digest.clone(),
            origin: self.origin,
            checkpoint: self.checkpoint.clone(),
            program: self.program.clone(),
            length: self.length,
            sha256: self.sha256,
            chunks: self.chunks.clone(),
        }
    }
}

/// A revision `step` adds after `before`, its snapshot checked against its checksum.
fn assemble_revision<'a>(
    before: &[AssembledRevision<'a>],
    step: &Step<'a>,
    revision: &StepRevision<'a>,
) -> Result<AssembledRevision<'a>, PersistenceError> {
    let previous = before
        .last()
        .ok_or(PersistenceError::InvalidRevisionHistory)?;
    let mut chunks = Vec::with_capacity(revision.numbers.len() / 4);
    let mut hasher = Sha256::new();
    let mut length = 0usize;
    for number in revision.numbers.chunks_exact(4) {
        let number = u32::from_le_bytes(number.try_into().expect("four bytes"));
        let chunk = if number & STEP_CHUNK_INLINE == 0 {
            previous.chunks.get(index(number))
        } else {
            step.chunks.get(index(number & !STEP_CHUNK_INLINE))
        }
        .copied()
        .ok_or(PersistenceError::InvalidRevisionHistory)?;
        length += chunk.len();
        hasher.update(chunk);
        chunks.push(chunk);
    }
    if length != revision.length || <[u8; 32]>::from(hasher.finalize()) != revision.sha256 {
        return Err(PersistenceError::HistoryChecksumMismatch);
    }
    let program = match revision.program {
        StepProgram::None => None,
        StepProgram::Inline(bytes) => Some(Cow::Borrowed(bytes)),
        StepProgram::AsPrevious => Some(
            previous
                .program
                .clone()
                .ok_or(PersistenceError::InvalidRevisionHistory)?,
        ),
    };
    Ok(AssembledRevision {
        id: revision.id,
        batch_digest: revision.batch_digest.clone(),
        origin: revision.origin,
        checkpoint: revision.checkpoint.clone(),
        program,
        length: revision.length,
        sha256: revision.sha256,
        chunks,
    })
}

/// `history` as a native file with the base's blobs and extensions.
fn assemble_file(
    base_entries: &[ContainerEntry<'_>],
    history: &History<AssembledRevision<'_>>,
) -> Result<Vec<u8>, PersistenceError> {
    let current = &history.revisions[history.cursor];
    let document = current.chunks.concat();
    if document.len() != current.length || crate::graph::sha256_bytes(&document) != current.sha256 {
        return Err(PersistenceError::HistoryChecksumMismatch);
    }
    // A chunk is one slice of the checkpoint however many revisions name it.
    let key = |chunk: &[u8]| (chunk.as_ptr().addr(), chunk.len());
    let mut numbers = BTreeMap::<(usize, usize), u32>::new();
    let mut table = Vec::new();
    let mut offset = 0usize;
    for chunk in &current.chunks {
        if !numbers.contains_key(&key(chunk)) {
            numbers.insert(key(chunk), u32_count(numbers.len())?);
            push_u8(&mut table, HISTORY_CHUNK_IN_DOCUMENT);
            push_u32(&mut table, u32_count(offset)?);
            push_u32(&mut table, u32_count(chunk.len())?);
        }
        offset += chunk.len();
    }
    let inline = |table: &mut Vec<u8>,
                  numbers: &mut BTreeMap<(usize, usize), u32>,
                  chunk: &[u8]|
     -> Result<u32, PersistenceError> {
        if let Some(number) = numbers.get(&key(chunk)) {
            return Ok(*number);
        }
        let number = u32_count(numbers.len())?;
        push_u8(table, HISTORY_CHUNK_INLINE);
        push_u32(table, u32_count(chunk.len())?);
        table.extend_from_slice(chunk);
        numbers.insert(key(chunk), number);
        Ok(number)
    };
    let mut programs = BTreeMap::<[u8; 32], u32>::new();
    let mut records = Vec::new();
    for (index, revision) in history.revisions.iter().enumerate() {
        write_revision_fields(
            &mut records,
            revision.id,
            &revision.batch_digest,
            revision.origin,
            revision.checkpoint.as_deref(),
        )?;
        match &revision.program {
            None => push_u8(&mut records, 0),
            Some(program)
                if index > 0 && history.revisions[index - 1].program.as_ref() == Some(program) =>
            {
                push_u8(&mut records, HISTORY_RULE_PROGRAM_AS_PREVIOUS);
            }
            Some(program) => {
                // A program is stored once as one chunk, however often it returns.
                let checksum = crate::graph::sha256_bytes(program);
                let number = match programs.get(&checksum) {
                    Some(number) => *number,
                    None => {
                        let number = inline(&mut table, &mut numbers, program)?;
                        programs.insert(checksum, number);
                        number
                    }
                };
                push_u8(&mut records, HISTORY_RULE_PROGRAM_IN_CHUNKS);
                push_u64(&mut records, u64_len(program.len())?);
                records.extend_from_slice(&checksum);
                push_u32(&mut records, 1);
                push_u32(&mut records, number);
            }
        }
        if index == history.cursor {
            push_u8(&mut records, HISTORY_SNAPSHOT_IS_DOCUMENT);
            continue;
        }
        push_u8(&mut records, HISTORY_SNAPSHOT_IN_CHUNKS);
        push_u64(&mut records, u64_len(revision.length)?);
        records.extend_from_slice(&revision.sha256);
        push_u32(&mut records, u32_count(revision.chunks.len())?);
        for chunk in &revision.chunks {
            let number = inline(&mut table, &mut numbers, chunk)?;
            push_u32(&mut records, number);
        }
    }
    let mut history_bin = Vec::with_capacity(32 + table.len() + records.len());
    history_bin.extend_from_slice(HISTORY_MAGIC);
    push_u16(&mut history_bin, HISTORY_SCHEMA);
    push_u32(&mut history_bin, u32_count(history.revisions.len())?);
    push_u32(&mut history_bin, u32_count(history.cursor)?);
    push_u64(&mut history_bin, history.next_revision_id);
    push_u32(&mut history_bin, u32_count(numbers.len())?);
    history_bin.extend_from_slice(&table);
    history_bin.extend_from_slice(&records);

    let mut entries = BTreeMap::new();
    for entry in base_entries {
        if !matches!(
            entry.path.as_str(),
            "document.bin" | "history.bin" | PROGRAM_ENTRY
        ) {
            entries.insert(entry.path.clone(), (entry.required, entry.content.to_vec()));
        }
    }
    entries.insert("document.bin".to_owned(), (true, document));
    entries.insert("history.bin".to_owned(), (true, history_bin));
    if let Some(program) = &current.program {
        entries.insert(PROGRAM_ENTRY.to_owned(), (true, program.to_vec()));
    }
    encode_container(entries)
}
