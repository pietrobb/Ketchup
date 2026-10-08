//! History compares complete CBOR values, not definite versus indefinite map lengths.
use super::{MAGIC, PersistenceError, Snapshot, snapshot_codec};

pub(super) enum Encoding {
    /// The bytes are what this version writes for the snapshot.
    Canonical(Vec<u8>),
    /// The same CBOR values with another map-length encoding.
    Equivalent,
    Different,
}

pub(super) fn compare(snapshot: &Snapshot, bytes: &[u8]) -> Result<Encoding, PersistenceError> {
    let canonical = snapshot_codec::encode(snapshot);
    if canonical == bytes {
        return Ok(Encoding::Canonical(canonical));
    }
    // The caller has already checked the snapshot envelope and payload checksum.
    let offset = MAGIC.len() + 2 + 32;
    let mut payload = &bytes[offset..];
    let stored: ciborium::Value = ciborium::from_reader(&mut payload)
        .map_err(|error| PersistenceError::InvalidPayload(error.to_string()))?;
    let expected: ciborium::Value = ciborium::from_reader(&canonical[offset..])
        .expect("a newly encoded snapshot contains CBOR");
    Ok(if payload.is_empty() && stored == expected {
        Encoding::Equivalent
    } else {
        Encoding::Different
    })
}

#[cfg(test)]
mod tests {
    use super::super::*;

    fn indefinite_snapshot(snapshot: &Snapshot) -> Vec<u8> {
        #[derive(serde::Serialize)]
        struct OldSnapshot<'a> {
            revision_id: u64,
            product: &'a crate::document::ProductModel,
        }
        let mut payload = Vec::new();
        ciborium::into_writer(
            &OldSnapshot {
                revision_id: snapshot.revision_id(),
                product: snapshot.product(),
            },
            &mut payload,
        )
        .unwrap();
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&CURRENT_SCHEMA.to_le_bytes());
        bytes.extend_from_slice(&crate::graph::sha256_bytes(&payload));
        bytes.extend_from_slice(&payload);
        bytes
    }

    /// A file whose history an older writer wrote in format 3, every revision whole,
    /// with `snapshot` as the bytes of the current revision.
    fn with_history_snapshot(document: &DocumentStore, snapshot: &[u8]) -> Vec<u8> {
        let mut history = HISTORY_MAGIC.to_vec();
        push_u16(&mut history, 3);
        push_u32(&mut history, document.revision_count() as u32);
        push_u32(&mut history, document.history_cursor() as u32);
        push_u64(&mut history, document.next_revision_id());
        for (index, revision) in document.revision_history().enumerate() {
            write_revision_metadata(&mut history, revision, None).unwrap();
            let bytes = if index == document.history_cursor() {
                snapshot.to_vec()
            } else {
                save(revision.snapshot())
            };
            push_u64(&mut history, bytes.len() as u64);
            history.extend_from_slice(&crate::graph::sha256_bytes(&bytes));
            history.extend_from_slice(&bytes);
        }
        save_container_entries(
            &encoded_snapshot(&document.current()),
            &ContainerData::default(),
            BTreeSet::new(),
            Some(history),
        )
        .unwrap()
    }

    #[test]
    fn flattened_writer_history_reopens_with_support_and_undo_redo() {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetFloorHeight {
                z_mm: Some(250.),
            }]))
            .unwrap();
        let old = indefinite_snapshot(&document.current());
        assert_ne!(old, save(&document.current()));
        let bytes = with_history_snapshot(&document, &old);
        let mut reopened = load(&bytes).unwrap().into_editable().ok().unwrap();
        assert_eq!(reopened.current().floor_z_mm(), Some(250.));
        reopened.undo().unwrap();
        assert_eq!(reopened.current().floor_z_mm(), None);
        reopened.redo().unwrap();
        assert_eq!(reopened.current().floor_z_mm(), Some(250.));
        let saved = save_document_store(&reopened, &ContainerData::default()).unwrap();
        assert_eq!(load(&saved).unwrap().snapshot().floor_z_mm(), Some(250.));
    }

    /// The document opens with only its current revision and names why.
    fn opens_without_history(bytes: &[u8], reason: &str) {
        let LoadOutcome::Editable {
            document, audit, ..
        } = load(bytes).unwrap()
        else {
            panic!("editable document")
        };
        assert_eq!(document.revision_count(), 1);
        assert_eq!(audit.history_discarded.as_deref(), Some(reason));
    }

    #[test]
    fn history_with_unknown_fields_trailing_payload_or_bad_checksum_is_discarded() {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::SetFloorHeight {
                z_mm: Some(250.),
            }]))
            .unwrap();
        let old = indefinite_snapshot(&document.current());
        let offset = MAGIC.len() + 2 + 32;
        let mut value: ciborium::Value = ciborium::from_reader(&old[offset..]).unwrap();
        let ciborium::Value::Map(fields) = &mut value else {
            panic!("snapshot map")
        };
        fields.push(("unknown".into(), true.into()));
        let mut payload = Vec::new();
        ciborium::into_writer(&value, &mut payload).unwrap();
        for payload in [payload, [old[offset..].to_vec(), vec![0]].concat()] {
            let mut changed = old[..MAGIC.len() + 2].to_vec();
            changed.extend_from_slice(&crate::graph::sha256_bytes(&payload));
            changed.extend_from_slice(&payload);
            opens_without_history(
                &with_history_snapshot(&document, &changed),
                "revision history is invalid",
            );
        }
        let mut corrupt = old;
        corrupt[offset] ^= 1;
        opens_without_history(
            &with_history_snapshot(&document, &corrupt),
            &PersistenceError::ChecksumMismatch.to_string(),
        );
    }
}
