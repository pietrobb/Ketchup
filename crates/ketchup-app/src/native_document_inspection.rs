use std::{collections::BTreeSet, path::Path};

use ketchup_core::document::FeatureKind;

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

pub fn inspect_native_document(path: &Path) -> Result<NativeDocumentInspection, String> {
    let container_bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let loaded = ketchup_core::persistence::load_file(path).map_err(|error| error.to_string())?;
    if loaded.source_schema() != ketchup_core::persistence::CURRENT_SCHEMA
        || loaded.disposition() != ketchup_core::persistence::LoadDisposition::EditableLossless
    {
        return Err("document is not a lossless current-schema document".to_owned());
    }
    let snapshot = loaded.snapshot();
    let profiles = snapshot
        .features()
        .filter(|feature| {
            matches!(
                feature.kind(),
                FeatureKind::Profile { .. } | FeatureKind::SegmentProfile { .. }
            )
        })
        .count();
    let extrusions = snapshot
        .features()
        .filter(|feature| matches!(feature.kind(), FeatureKind::Extrusion { .. }))
        .count();
    let profile_extrusion_definition_ids = snapshot
        .definitions()
        .filter(|definition| {
            definition.feature_ids().iter().any(|feature_id| {
                let Some(feature) = snapshot.feature(*feature_id) else {
                    return false;
                };
                let FeatureKind::Extrusion { profile, .. } = feature.kind() else {
                    return false;
                };
                snapshot.feature(*profile).is_some_and(|profile_feature| {
                    profile_feature.definition_id() == definition.id()
                        && matches!(
                            profile_feature.kind(),
                            FeatureKind::Profile { .. }
                                | FeatureKind::SegmentProfile { closed: true, .. }
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
        container_sha256: ketchup_core::graph::sha256_hex(&container_bytes),
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
