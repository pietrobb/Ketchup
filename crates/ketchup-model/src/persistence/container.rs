//! The framing of a native container: its entries, their flag bytes and
//! checksums.

use std::collections::BTreeMap;

use super::{
    CONTAINER_MAGIC, CONTAINER_SCHEMA, ENTRY_LIMIT_OF_0_2_0, LARGE_CONTAINER_SCHEMA,
    MAX_CONTAINER_ENTRIES, NEWER_EXTENSION_CONTAINER_SCHEMA, PersistenceError, Reader, push_string,
    push_u8, push_u16, push_u32, push_u64, u32_count, u64_len, validate_container_path,
};

/// How a container keeps an entry, its flag byte.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EntryFlag {
    /// A reader that does not know the entry keeps it as it is.
    Optional = 0,
    /// A reader that does not know the entry opens the file for review only.
    Required = 1,
    /// Optional, and a released Kečup would lose part of it: only in
    /// [`NEWER_EXTENSION_CONTAINER_SCHEMA`].
    LostByReleasedReaders = 2,
}

impl EntryFlag {
    pub(super) fn of(required: bool) -> Self {
        if required {
            Self::Required
        } else {
            Self::Optional
        }
    }

    pub(super) fn read(byte: u8, schema: u16) -> Result<Self, PersistenceError> {
        match byte {
            0 => Ok(Self::Optional),
            1 => Ok(Self::Required),
            2 if schema == NEWER_EXTENSION_CONTAINER_SCHEMA => Ok(Self::LostByReleasedReaders),
            value => Err(PersistenceError::InvalidBoolean(value)),
        }
    }

    /// The flag of a blob: optional, and lost by the releases where they would not draw
    /// it, so that they refuse its file as newer (review 2026-11-25 R1125og-DXF-1001).
    pub(super) fn of_blob(bytes: &[u8]) -> Self {
        if crate::underlay::UnderlayLines::newer_than_releases_draw(bytes) {
            Self::LostByReleasedReaders
        } else {
            Self::Optional
        }
    }

    /// Whether a blob may be kept with this flag.
    pub(super) const fn fits_blob(self) -> bool {
        !matches!(self, Self::Required)
    }

    /// The flag byte this entry is written with.
    pub(super) const fn byte(self) -> u8 {
        match self {
            Self::Optional => 0,
            Self::Required => 1,
            Self::LostByReleasedReaders => 2,
        }
    }
}

/// `entries` (path → flag, content) as a native container.
pub(super) fn encode_container(
    entries: BTreeMap<String, (EntryFlag, impl AsRef<[u8]>)>,
) -> Result<Vec<u8>, PersistenceError> {
    if entries.len() > MAX_CONTAINER_ENTRIES as usize {
        return Err(PersistenceError::ResourceLimit);
    }

    let large = entries
        .values()
        .any(|(_, bytes)| bytes.as_ref().len() > ENTRY_LIMIT_OF_0_2_0);
    let mut encoded = Vec::new();
    encoded.extend_from_slice(CONTAINER_MAGIC);
    let newer = entries
        .values()
        .any(|(flag, _)| *flag == EntryFlag::LostByReleasedReaders);
    push_u16(
        &mut encoded,
        if newer {
            NEWER_EXTENSION_CONTAINER_SCHEMA
        } else if large {
            LARGE_CONTAINER_SCHEMA
        } else {
            CONTAINER_SCHEMA
        },
    );
    push_u32(&mut encoded, u32_count(entries.len())?);
    for (path, (flag, bytes)) in entries {
        let bytes = bytes.as_ref();
        validate_container_path(&path)?;
        push_string(&mut encoded, &path)?;
        push_u8(&mut encoded, flag.byte());
        push_u64(&mut encoded, u64_len(bytes.len())?);
        encoded.extend_from_slice(&crate::graph::sha256_bytes(bytes));
        encoded.extend_from_slice(bytes);
    }
    Ok(encoded)
}

/// The entries of a native container whose framing and checksums hold.
pub(super) fn container_entries(
    bytes: &[u8],
) -> Result<BTreeMap<String, (EntryFlag, Vec<u8>)>, PersistenceError> {
    let mut entries = BTreeMap::<String, (EntryFlag, Vec<u8>)>::new();
    for entry in container_directory(bytes)? {
        if crate::graph::sha256_bytes(entry.content) != entry.checksum {
            return Err(PersistenceError::ContainerChecksumMismatch(entry.path));
        }
        if entries
            .insert(entry.path, (entry.flag, entry.content.to_vec()))
            .is_some()
        {
            return Err(PersistenceError::DuplicateContainerEntry);
        }
    }
    Ok(entries)
}

/// One entry of a native container as its framing names it.
pub(super) struct ContainerEntry<'a> {
    pub(super) path: String,
    pub(super) flag: EntryFlag,
    pub(super) checksum: [u8; 32],
    pub(super) content: &'a [u8],
}

/// The entries of a native container whose framing holds, their checksums not
/// verified: for bytes whose file identity was already checked.
pub(super) fn container_directory(
    bytes: &[u8],
) -> Result<Vec<ContainerEntry<'_>>, PersistenceError> {
    let mut reader = Reader::new(bytes);
    if reader.take(CONTAINER_MAGIC.len())? != CONTAINER_MAGIC {
        return Err(PersistenceError::InvalidContainerMagic);
    }
    let schema = reader.u16()?;
    if ![
        CONTAINER_SCHEMA,
        LARGE_CONTAINER_SCHEMA,
        NEWER_EXTENSION_CONTAINER_SCHEMA,
    ]
    .contains(&schema)
    {
        return Err(PersistenceError::UnsupportedContainerSchema(schema));
    }
    let entry_count = reader.count_with_limit(MAX_CONTAINER_ENTRIES)?;
    let mut entries = Vec::new();
    for _ in 0..entry_count {
        let path = reader.string()?;
        validate_container_path(&path)?;
        let flag = EntryFlag::read(reader.u8()?, schema)?;
        let length = usize::try_from(reader.u64()?)
            .map_err(|_out_of_range: std::num::TryFromIntError| PersistenceError::LengthOverflow)?;
        // Every stored length fits in the rest of the file, its checksum first.
        if length > (bytes.len() - reader.cursor).saturating_sub(32) {
            return Err(PersistenceError::Truncated);
        }
        let checksum: [u8; 32] = reader
            .take(32)?
            .try_into()
            .map_err(|_wrong_length: std::array::TryFromSliceError| PersistenceError::Truncated)?;
        entries.push(ContainerEntry {
            path,
            flag,
            checksum,
            content: reader.take(length)?,
        });
    }
    if !reader.is_finished() {
        return Err(PersistenceError::TrailingBytes);
    }
    Ok(entries)
}
