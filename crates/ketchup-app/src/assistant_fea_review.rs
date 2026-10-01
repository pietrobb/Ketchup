//! Opening the FEA review the Assistant proposed: the request must target the single current
//! exact body, otherwise the Assistant receives a typed planning rejection it can repair.

use super::*;

impl KetchupApp {
    pub(crate) fn prepare_assistant_fea_review(
        &mut self,
        request: &AssistantFeaReviewRequest,
    ) -> AssistantPlanningResult<()> {
        let target = format!("occurrence:{}", request.occurrence_id);
        let rejected = |code, invariant: String, repair| {
            assistant_planning_rejection(code, "fea_review", &target, invariant, repair)
        };
        let retarget = "Refresh document context and target the single current exact body by its occurrence, definition and terminal feature ids.";
        request.validate().map_err(|error| {
            rejected(
                "planning.fea_request_invalid",
                error,
                "Send finite material, face, traction and deflection values inside their documented ranges.",
            )
        })?;
        let snapshot = self.document.current();
        let occurrence = snapshot
            .occurrence(OccurrenceId(request.occurrence_id))
            .filter(|occurrence| occurrence.definition_id() == DefinitionId(request.definition_id))
            .ok_or_else(|| {
                rejected(
                    "planning.fea_target_stale",
                    "The FEA occurrence is not current or does not use the requested definition."
                        .to_owned(),
                    retarget,
                )
            })?;
        let terminals = exact_body_terminal_features(&snapshot, occurrence.definition_id())
            .map_err(|error| {
                rejected("planning.fea_target_ambiguous", error.to_string(), retarget)
            })?;
        let terminal_features = terminals.values().copied().collect::<Vec<_>>();
        if terminal_features.as_slice() != [FeatureId(request.feature_id)] {
            return Err(rejected(
                "planning.fea_target_ambiguous",
                "The FEA feature is not the sole current exact body of its definition.".to_owned(),
                retarget,
            ));
        }
        self.reviews.fea_review_dialog = Some(FeaReviewDialog {
            definition_id: occurrence.definition_id(),
            feature_id: FeatureId(request.feature_id),
            occurrence_id: OccurrenceId(request.occurrence_id),
            case_id: request.case_id.clone(),
            youngs_modulus_mpa: request.youngs_modulus_mpa.to_string(),
            poisson_ratio: request.poisson_ratio.to_string(),
            yield_strength_mpa: request.yield_strength_mpa.to_string(),
            constrained_face_ordinals: request
                .constrained_face_ordinals
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(","),
            loaded_face_ordinal: request.loaded_face_ordinal.to_string(),
            traction_x_n_per_mm2: request.traction_local_n_per_mm2[0].to_string(),
            traction_y_n_per_mm2: request.traction_local_n_per_mm2[1].to_string(),
            traction_z_n_per_mm2: request.traction_local_n_per_mm2[2].to_string(),
            coarse_deflection_mm: request.coarse_deflection_mm.to_string(),
            fine_deflection_mm: request.fine_deflection_mm.to_string(),
            review: None,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fea_request(
        occurrence_id: u64,
        definition_id: u64,
        feature_id: u64,
    ) -> AssistantFeaReviewRequest {
        AssistantFeaReviewRequest {
            definition_id,
            feature_id,
            occurrence_id,
            case_id: "bracket".to_owned(),
            youngs_modulus_mpa: 210_000.0,
            poisson_ratio: 0.3,
            yield_strength_mpa: 250.0,
            constrained_face_ordinals: vec![1],
            loaded_face_ordinal: 2,
            traction_local_n_per_mm2: [0.0, 0.0, -1.0],
            coarse_deflection_mm: 1.0,
            fine_deflection_mm: 0.1,
        }
    }

    #[test]
    fn an_unusable_fea_review_target_is_a_typed_planning_rejection() {
        let mut app = KetchupApp::new();
        let mut invalid = fea_request(1, 1, 2);
        invalid.case_id.clear();
        for (request, code) in [
            (invalid, "planning.fea_request_invalid"),
            (fea_request(999, 1, 2), "planning.fea_target_stale"),
            (fea_request(1, 999, 2), "planning.fea_target_stale"),
            (fea_request(1, 1, 1), "planning.fea_target_ambiguous"),
        ] {
            let rejection = app.prepare_assistant_fea_review(&request).unwrap_err();
            assert_eq!(rejection.code, code);
            assert_eq!(rejection.operation, "fea_review");
            assert_eq!(
                rejection.target,
                format!("occurrence:{}", request.occurrence_id)
            );
            assert!(!rejection.repair_hint.is_empty());
            assert_eq!(rejection.validate(), Ok(()));
            assert!(app.reviews.fea_review_dialog.is_none());
        }
    }
}
