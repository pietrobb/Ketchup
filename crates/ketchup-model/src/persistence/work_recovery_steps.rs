//! Idea #49: the steps an edit of a saved document appends to its work-recovery
//! checkpoint instead of writing the whole document and Undo history again.
//!
//! A checkpoint is the whole native file written at the first edit (its base),
//! then one step per later edit: how many revisions at the start of the history
//! were dropped and how many after them stay, the Undo position, the id and
//! checksum of each revision added, and a `history.bin` ([`encode_history`]) of
//! the last kept revision and the added ones. The kept revision is that
//! history's document, so only the chunks it lacks are stored; it is written
//! without its program, and an added revision with that same program says so.
//! A step is a native container of one entry, so its checksum tells a step a
//! crash cut short. Opening assembles the base and its steps into one native
//! file and opens that like any other.
//!
//! The checkpoint is written whole when a step cannot name the change (no
//! revision is shared with it, blobs or extensions changed, a step does not
//! read) or once the steps outweigh the base.

use std::borrow::Cow;

use super::*;

const STEP_ENTRY: &str = "step.bin";
/// The entries of a file beside its blobs and extensions.
const CORE_ENTRIES: [&str; 3] = ["document.bin", "history.bin", PROGRAM_ENTRY];

/// A count or index the file names as `u32`, as the `usize` it indexes with.
fn index(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// The identity of the file a checkpoint was made from, its base and its steps.
fn split_checkpoint(bytes: &[u8]) -> Option<(FileIdentity, &[u8], &[u8])> {
    let mut reader = Reader::new(bytes);
    let magic = reader.take(WORK_RECOVERY_MAGIC.len()).ok()?;
    let schema = reader.u16().ok()?;
    let byte_len = reader.u64().ok()?;
    let sha256 = reader.take(32).ok()?.try_into().ok()?;
    let payload_len = usize::try_from(reader.u64().ok()?).ok()?;
    let payload = reader.take(payload_len).ok()?;
    let base = FileIdentity { byte_len, sha256 };
    (magic == WORK_RECOVERY_MAGIC && schema == WORK_RECOVERY_SCHEMA)
        .then(|| (base, payload, &bytes[reader.cursor..]))
}

/// The `document.bin` entry of a base and its parsed `history.bin`.
fn read_base<'e, 'a>(
    entries: &'e [ContainerEntry<'a>],
) -> Result<(&'e ContainerEntry<'a>, ParsedHistory<'a>), PersistenceError> {
    let entry = |path: &str| {
        entries
            .iter()
            .find(|entry| entry.path == path && entry.required)
    };
    let document = entry("document.bin").ok_or(PersistenceError::MissingDocumentEntry)?;
    let history = entry("history.bin").ok_or(PersistenceError::InvalidRevisionHistory)?;
    let history = read_history_records(history.content, document.content, HISTORY_BUDGET, false)?;
    Ok((document, history))
}

/// The blobs and extensions of a file.
fn side_of<'a>(entries: &[ContainerEntry<'a>]) -> BTreeMap<String, (bool, &'a [u8])> {
    entries
        .iter()
        .filter(|entry| !CORE_ENTRIES.contains(&entry.path.as_str()))
        .map(|entry| (entry.path.clone(), (entry.required, entry.content)))
        .collect()
}

/// Blobs and extensions (path → required, checksum) as a step compares them: a
/// blob's path names the SHA-256 of its content, so only an extension is hashed.
fn compared(side: BTreeMap<String, (bool, &[u8])>) -> BTreeMap<String, (bool, Option<[u8; 32]>)> {
    side.into_iter()
        .map(|(path, (required, content))| {
            let checksum =
                (!path.starts_with("blobs/")).then(|| crate::graph::sha256_bytes(content));
            (path, (required, checksum))
        })
        .collect()
}

/// The steps in `bytes` whose framing and checksum hold, then why the first one
/// that does not was left out with every step after it.
fn read_steps(bytes: &[u8]) -> (Vec<Vec<u8>>, Option<PersistenceError>) {
    let mut reader = Reader::new(bytes);
    let mut steps = Vec::new();
    while !reader.is_finished() {
        let mut step = || {
            let length = usize::try_from(reader.u64()?)
                .map_err(|_: std::num::TryFromIntError| PersistenceError::LengthOverflow)?;
            let mut entries = container_entries(reader.take(length)?)?;
            match entries.remove(STEP_ENTRY) {
                Some((true, content)) if entries.is_empty() => Ok(content),
                _ => Err(PersistenceError::InvalidRevisionHistory),
            }
        };
        match step() {
            Ok(step) => steps.push(step),
            Err(error) => return (steps, Some(error)),
        }
    }
    (steps, None)
}

/// A revision as a checkpoint names it: its id and the checksum of its snapshot.
type Held = (u64, [u8; 32]);

struct Step<'a> {
    skip: usize,
    keep: usize,
    cursor: usize,
    /// The revisions added, each with whether its program is the kept one's.
    added: Vec<(Held, bool)>,
    history: &'a [u8],
}

impl<'a> Step<'a> {
    fn read(bytes: &'a [u8]) -> Result<Self, PersistenceError> {
        let mut reader = Reader::new(bytes);
        let (skip, keep, cursor) = (reader.u32()?, reader.u32()?, reader.u32()?);
        let count = reader.count_with_limit(FILE_HISTORY_REVISIONS)?;
        let mut added = Vec::with_capacity(index(count));
        for _ in 0..count {
            let id = reader.u64()?;
            let checksum = reader.take(32)?.try_into().expect("32 bytes");
            added.push(((id, checksum), reader.boolean()?));
        }
        let (skip, keep, cursor) = (index(skip), index(keep), index(cursor));
        let history = &bytes[reader.cursor..];
        Ok(Self {
            skip,
            keep,
            cursor,
            added,
            history,
        })
    }

    /// The index of the last kept revision of a history of `len` revisions.
    fn kept(&self, len: usize) -> Result<usize, PersistenceError> {
        (self.keep > 0 && self.skip.saturating_add(self.keep) <= len)
            .then(|| self.skip + self.keep - 1)
            .ok_or(PersistenceError::InvalidRevisionHistory)
    }

    /// `history` with this step applied, `added` after the kept revisions.
    fn apply<T>(&self, history: &mut Vec<T>, added: Vec<T>) -> Result<(), PersistenceError> {
        self.kept(history.len())?;
        if added.len() != self.added.len() || self.cursor >= self.keep + added.len() {
            return Err(PersistenceError::InvalidRevisionHistory);
        }
        history.drain(..self.skip);
        history.truncate(self.keep);
        history.extend(added);
        Ok(())
    }
}

/// What a step is planned against, so an edit neither reads the checkpoint nor
/// copies its blobs (review 2026-10-21 LIM-1).
#[derive(Clone, Debug)]
pub(super) struct CheckpointPlan {
    base_identity: FileIdentity,
    len: usize,
    payload_len: usize,
    held: Vec<Held>,
    side: BTreeMap<String, (bool, Option<[u8; 32]>)>,
}

/// The plan of the checkpoint `bytes`, `None` where only a whole one can follow.
pub(super) fn read_plan(bytes: &[u8]) -> Option<CheckpointPlan> {
    let (base_identity, payload, steps) = split_checkpoint(bytes)?;
    let entries = container_directory(payload).ok()?;
    let (base_document, base) = read_base(&entries).ok()?;
    let mut held = Vec::<Held>::new();
    for record in &base.records {
        let checksum = record.encoded_snapshot.checksum(base_document.checksum);
        held.push((record.revision_id, checksum));
    }
    let (steps, None) = read_steps(steps) else {
        return None;
    };
    for step in &steps {
        let step = Step::read(step).ok()?;
        let added = step.added.iter().map(|(held, _)| *held).collect();
        step.apply(&mut held, added).ok()?;
    }
    Some(CheckpointPlan {
        base_identity,
        len: bytes.len(),
        payload_len: payload.len(),
        held,
        side: compared(side_of(&entries)),
    })
}

/// The step that brings the checkpoint planned as `plan` to `document`, with
/// the plan of the checkpoint it makes, or `None` when only a whole checkpoint
/// can.
pub(super) fn plan_step(
    plan: &CheckpointPlan,
    document: &DocumentStore,
    container: &ContainerData,
    base_identity: FileIdentity,
) -> Option<(Vec<u8>, CheckpointPlan)> {
    let side = side_entries(container, &history_imported_sources(document)).ok()?;
    if plan.base_identity != base_identity || compared(side) != plan.side {
        return None;
    }
    let revisions = document.revision_history().collect::<Vec<_>>();
    let current = |revision: &Revision| (revision.id(), revision.encoded().sha256);
    let held = &plan.held;
    let skip = held
        .iter()
        .position(|held| *held == current(revisions[0]))?;
    let keep = held[skip..]
        .iter()
        .zip(&revisions)
        .take_while(|(held, revision)| **held == current(revision))
        .count();
    let (cursor, added) = (document.history_cursor(), revisions.len() - keep);
    let mut step = Vec::new();
    for value in [skip, keep, cursor, added] {
        push_u32(&mut step, u32_count(value).ok()?);
    }
    let kept = revisions[keep - 1];
    let mut written = vec![Assembled::of_revision(kept, false).ok()?];
    for revision in &revisions[keep..] {
        let as_kept = revision.same_program(kept);
        let (id, checksum) = current(revision);
        push_u64(&mut step, id);
        step.extend_from_slice(&checksum);
        push_u8(&mut step, u8::from(as_kept));
        written.push(Assembled::of_revision(revision, !as_kept).ok()?);
    }
    let written = written.iter().collect::<Vec<_>>();
    let next_revision_id = document.next_revision_id();
    let kept = &written[0].snapshot;
    let history = encode_history(&written, 0, next_revision_id, kept, HISTORY_BUDGET).ok()?;
    step.extend_from_slice(&history);
    let step = encode_container(BTreeMap::from([(STEP_ENTRY.to_owned(), (true, step))])).ok()?;
    // The steps stay lighter than the base.
    let len = plan.len + 8 + step.len();
    (len - plan.payload_len <= plan.payload_len).then(|| {
        let next = CheckpointPlan {
            len,
            held: revisions.iter().map(|revision| current(revision)).collect(),
            ..plan.clone()
        };
        (step, next)
    })
}

/// A revision as a checkpoint holds it, with its program JSON.
struct Assembled {
    id: u64,
    batch_digest: String,
    origin: RevisionOrigin,
    checkpoint: Option<String>,
    program: Option<Arc<[u8]>>,
    snapshot: Arc<crate::document::EncodedSnapshot>,
}

impl HistoryRevision for Assembled {
    fn fields(&self) -> (u64, &str, RevisionOrigin, Option<&str>) {
        let checkpoint = self.checkpoint.as_deref();
        (self.id, &self.batch_digest, self.origin, checkpoint)
    }

    fn same_program(&self, previous: &Self) -> bool {
        self.program.is_some() && self.program == previous.program
    }

    fn program_json(&self) -> Result<Option<Cow<'_, [u8]>>, PersistenceError> {
        Ok(self.program.as_deref().map(Cow::Borrowed))
    }

    fn encoded(&self) -> Arc<crate::document::EncodedSnapshot> {
        Arc::clone(&self.snapshot)
    }
}

impl Assembled {
    fn of_revision(revision: &Revision, with_program: bool) -> Result<Self, PersistenceError> {
        let program = if with_program {
            revision.program_json()?.map(|json| Arc::from(&*json))
        } else {
            None
        };
        let (id, batch_digest, origin, checkpoint) = revision.fields();
        Ok(Self {
            id,
            batch_digest: batch_digest.to_owned(),
            origin,
            checkpoint: checkpoint.map(str::to_owned),
            program,
            snapshot: revision.encoded(),
        })
    }

    /// `record` of a history with `chunks` whose document has the checksum `document`.
    fn of_record(
        record: &HistoryRecord<'_>,
        chunks: &[&[u8]],
        document: [u8; 32],
    ) -> Result<Self, PersistenceError> {
        let program = record.rule_program.map(|program| program.assemble(chunks));
        let snapshot = &record.encoded_snapshot;
        Ok(Self {
            id: record.revision_id,
            batch_digest: record.batch_digest.clone(),
            origin: record.origin,
            checkpoint: record.checkpoint.clone(),
            program: program.transpose()?.map(|json| Arc::from(&*json)),
            snapshot: Arc::new(crate::document::EncodedSnapshot {
                sha256: snapshot.checksum(document),
                bytes: snapshot.assemble(chunks)?.into_owned(),
                chunks: std::sync::OnceLock::new(),
            }),
        })
    }
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
    verify_checksums(payload)?;
    let entries = container_directory(payload)?;
    let (document, base) = read_base(&entries)?;
    let mut revisions = Vec::with_capacity(base.records.len());
    for record in &base.records {
        revisions.push(Assembled::of_record(
            record,
            &base.chunks,
            document.checksum,
        )?);
    }
    let (mut cursor, mut next_revision_id) = (base.cursor, base.next_revision_id);
    for (number, step) in steps.iter().enumerate() {
        let applied = Step::read(step).and_then(|step| {
            let kept = &revisions[step.kept(revisions.len())?];
            let history =
                read_history_records(step.history, &kept.snapshot.bytes, HISTORY_BUDGET, false)?;
            let mut added = Vec::with_capacity(step.added.len());
            for (record, (held, as_kept)) in history.records.iter().skip(1).zip(&step.added) {
                let mut revision = Assembled::of_record(record, &history.chunks, [0; 32])?;
                if *as_kept {
                    revision.program.clone_from(&kept.program);
                }
                if (revision.id, revision.snapshot.sha256) != *held {
                    return Err(PersistenceError::InvalidRevisionHistory);
                }
                added.push(revision);
            }
            if history.records[0].revision_id != kept.id {
                return Err(PersistenceError::InvalidRevisionHistory);
            }
            let next = history.next_revision_id;
            step.apply(&mut revisions, added)?;
            Ok((step.cursor, next))
        });
        match applied {
            Ok(applied) => (cursor, next_revision_id) = applied,
            Err(error) => {
                damaged = Some((number, error));
                break;
            }
        }
    }

    let revisions = revisions.iter().collect::<Vec<_>>();
    let current = revisions[cursor];
    let file = |revisions: &[&Assembled], cursor: usize| {
        let snapshot = &current.snapshot;
        let history = encode_history(
            revisions,
            cursor,
            next_revision_id,
            snapshot,
            HISTORY_BUDGET,
        );
        let history = history?;
        let mut file = side_of(&entries);
        file.insert("document.bin".to_owned(), (true, &snapshot.bytes[..]));
        file.insert("history.bin".to_owned(), (true, &history[..]));
        if let Some(program) = &current.program {
            file.insert(PROGRAM_ENTRY.to_owned(), (true, &program[..]));
        }
        encode_container(file)
    };
    // A history that does not fit keeps only the current revision, as a whole
    // checkpoint does.
    let file = match file(&revisions, cursor) {
        Err(PersistenceError::ResourceLimit | PersistenceError::TooLarge { .. }) => {
            file(&[current], 0)?
        }
        file => file?,
    };
    Ok((file, damaged.map(damage_reason)))
}

fn damage_reason((number, error): (usize, PersistenceError)) -> String {
    format!("step {} of the checkpoint is damaged: {error}", number + 1)
}
