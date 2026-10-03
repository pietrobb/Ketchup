//! History compares complete CBOR values, not definite versus indefinite map lengths.
use super::{MAGIC, PersistenceError, Snapshot, snapshot_codec};

pub(super) fn matches(snapshot: &Snapshot, bytes: &[u8]) -> Result<bool, PersistenceError> {
    let canonical = snapshot_codec::encode(snapshot);
    if canonical == bytes {
        return Ok(true);
    }
    // The caller has already checked the snapshot envelope and payload checksum.
    let offset = MAGIC.len() + 2 + 32;
    let mut payload = &bytes[offset..];
    let stored: ciborium::Value = ciborium::from_reader(&mut payload)
        .map_err(|error| PersistenceError::InvalidPayload(error.to_string()))?;
    let expected: ciborium::Value = ciborium::from_reader(&canonical[offset..])
        .expect("a newly encoded snapshot contains CBOR");
    Ok(payload.is_empty() && stored == expected)
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

    fn with_history_snapshot(document: &DocumentStore, snapshot: &[u8]) -> Vec<u8> {
        let mut history = encode_revision_history(document).unwrap();
        let canonical = save(&document.current());
        let start = history
            .windows(canonical.len())
            .position(|bytes| bytes == canonical)
            .unwrap();
        history[start - 40..start - 32].copy_from_slice(&(snapshot.len() as u64).to_le_bytes());
        history[start - 32..start].copy_from_slice(&crate::graph::sha256_bytes(snapshot));
        history.splice(start..start + canonical.len(), snapshot.iter().copied());
        save_container_entries(
            &document.current(),
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

    #[test]
    fn history_still_rejects_unknown_fields_trailing_payload_and_bad_checksum() {
        let document = DocumentStore::new();
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
            assert!(matches!(
                load(&with_history_snapshot(&document, &changed)),
                Err(PersistenceError::InvalidRevisionHistory)
            ));
        }
        let mut corrupt = old;
        corrupt[offset] ^= 1;
        assert!(matches!(
            load(&with_history_snapshot(&document, &corrupt)),
            Err(PersistenceError::ChecksumMismatch)
        ));
    }
}
