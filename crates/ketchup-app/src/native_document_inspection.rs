use std::{collections::BTreeSet, path::Path};

use ketchup_geometry::sketch::{PadOperation, PadProfile, PadSpec};
use ketchup_model::document::FeatureKind;
use ketchup_model::persistence::{
    CURRENT_SCHEMA, FilePersistenceError, LoadAudit, LoadDisposition, load_file_with_source,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeDocumentInspection {
    pub schema_version: u16,
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub container_sha256: String,
    pub definitions: usize,
    pub root_occurrences: usize,
    pub profiles: usize,
    pub extrusions: usize,
    pub profile_extrusion_definitions: usize,
    pub visible_profile_extrusion_root_occurrences: usize,
}

impl NativeDocumentInspection {
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema_version\":{},\"document_id\":{},\"revision\":{},\"canonical_digest\":\"{}\",\"container_sha256\":\"{}\",\"definitions\":{},\"root_occurrences\":{},\"profiles\":{},\"extrusions\":{},\"profile_extrusion_definitions\":{},\"visible_profile_extrusion_root_occurrences\":{}}}",
            self.schema_version,
            self.document_id,
            self.revision,
            self.canonical_digest,
            self.container_sha256,
            self.definitions,
            self.root_occurrences,
            self.profiles,
            self.extrusions,
            self.profile_extrusion_definitions,
            self.visible_profile_extrusion_root_occurrences
        )
    }
}

#[derive(Debug)]
pub enum NativeDocumentInspectionError {
    Load(FilePersistenceError),
    /// Inspection counts what a current-schema file stores without loss; the audit names
    /// the source schema and what a migration would change or lose.
    NotCurrentLossless(Box<LoadAudit>),
}

impl std::fmt::Display for NativeDocumentInspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Load(error) => error.fmt(formatter),
            Self::NotCurrentLossless(audit) => write!(
                formatter,
                "document is not a lossless current-schema document (schema {}, current {CURRENT_SCHEMA}; {} migration losses, {} unknown extensions)",
                audit.source_schema,
                audit.migration_losses.len(),
                audit.unknown_extensions.len()
            ),
        }
    }
}

impl std::error::Error for NativeDocumentInspectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Load(error) => Some(error),
            Self::NotCurrentLossless(_) => None,
        }
    }
}

pub fn inspect_native_document(
    path: &Path,
) -> Result<NativeDocumentInspection, NativeDocumentInspectionError> {
    let loaded_file = load_file_with_source(path).map_err(NativeDocumentInspectionError::Load)?;
    let container_sha256 = ketchup_model::graph::sha256_hex(loaded_file.source_bytes());
    let loaded = loaded_file.outcome();
    if loaded.source_schema() != CURRENT_SCHEMA
        || loaded.disposition() != LoadDisposition::EditableLossless
    {
        return Err(NativeDocumentInspectionError::NotCurrentLossless(Box::new(
            loaded.audit().clone(),
        )));
    }
    let snapshot = loaded.snapshot();
    let profiles = snapshot
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::Profile { .. }))
        .count();
    let extrusions = snapshot
        .features()
        .filter(|feature| {
            matches!(
                feature.kind(),
                FeatureKind::Pad(PadSpec {
                    operation: PadOperation::NewBody,
                    ..
                })
            )
        })
        .count();
    let profile_extrusion_definition_ids = snapshot
        .definitions()
        .filter(|definition| {
            definition.feature_ids().iter().any(|feature_id| {
                let Some(feature) = snapshot.feature(*feature_id) else {
                    return false;
                };
                let FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(profile),
                    operation: PadOperation::NewBody,
                    ..
                }) = feature.kind()
                else {
                    return false;
                };
                snapshot.feature(*profile).is_some_and(|profile_feature| {
                    profile_feature.definition_id() == definition.id()
                        && matches!(
                            profile_feature.kind(),
                            FeatureKind::Profile { closed: true, .. }
                        )
                })
            })
        })
        .map(|definition| definition.id())
        .collect::<BTreeSet<_>>();
    let visible_profile_extrusion_root_occurrences = snapshot
        .scene_query()
        .into_iter()
        .filter(|occurrence| {
            occurrence.visible
                && occurrence.instance_path.is_root()
                && profile_extrusion_definition_ids.contains(&occurrence.definition_id)
        })
        .count();
    Ok(NativeDocumentInspection {
        schema_version: loaded.source_schema(),
        document_id: snapshot.document_id().0,
        revision: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
        container_sha256,
        definitions: snapshot.definitions().count(),
        root_occurrences: snapshot.occurrences().count(),
        profiles,
        extrusions,
        profile_extrusion_definitions: profile_extrusion_definition_ids.len(),
        visible_profile_extrusion_root_occurrences,
    })
}

#[cfg(test)]
mod tests {
    use super::NativeDocumentInspection;

    #[test]
    fn json_contract_is_exact_and_ordered() {
        let inspection = NativeDocumentInspection {
            schema_version: 7,
            document_id: 11,
            revision: 13,
            canonical_digest: "canonical-digest".to_owned(),
            container_sha256: "container-sha256".to_owned(),
            definitions: 17,
            root_occurrences: 19,
            profiles: 23,
            extrusions: 29,
            profile_extrusion_definitions: 31,
            visible_profile_extrusion_root_occurrences: 37,
        };

        assert_eq!(
            inspection.to_json(),
            "{\"schema_version\":7,\"document_id\":11,\"revision\":13,\"canonical_digest\":\"canonical-digest\",\"container_sha256\":\"container-sha256\",\"definitions\":17,\"root_occurrences\":19,\"profiles\":23,\"extrusions\":29,\"profile_extrusion_definitions\":31,\"visible_profile_extrusion_root_occurrences\":37}"
        );
    }
}
