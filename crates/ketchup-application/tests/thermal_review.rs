//! A steady thermal review of a real exact part: the conductivity comes from
//! the part's material in the library rules with its standard, two finer
//! meshes agree, and a part whose material has no conductivity is refused.

use ketchup_analysis::fea_thermal::ThermalSolveSettings;
use ketchup_application::evaluation::exact_worker_candidates;
use ketchup_application::thermal_workflow::{
    ExactThermalFaceTemperature, ExactVolumeMeshWireOptions, ThermalReviewError,
    ThermalReviewWorkflow, ThermalStudyRequest,
};
use ketchup_application::validation_rules::ValidationRules;
use ketchup_model::document::*;
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_model::validation::MATERIAL_DIMENSION_V1;
use ketchup_scheduler::ExactWorkerSupervisor;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

const RADIUS_MM: f64 = 10.0;
const HEIGHT_MM: f64 = 20.0;

fn worker_path() -> PathBuf {
    exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test")
}

/// A Ø20 × 20 mm cylinder standing on the XY plane.
fn cylinder() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Heated pin".into(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Section".into(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::CircularArc {
                            start_mm: [RADIUS_MM, 0.0],
                            end_mm: [-RADIUS_MM, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [-RADIUS_MM, 0.0],
                            end_mm: [RADIUS_MM, 0.0],
                            center_mm: [0.0, 0.0],
                            clockwise: false,
                        },
                    ],
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Pin".into(),
                kind: FeatureKind::extrusion(
                    FeatureId(1),
                    Dimension::new("20", HEIGHT_MM).unwrap(),
                ),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Pin".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document
}

fn classify(document: &mut DocumentStore, material: &str) {
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(1),
                name: MATERIAL_DIMENSION_V1.into(),
                categories: vec![(ClassificationCategoryId(1), material.into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: OccurrenceId(1),
                dimension_id: ClassificationDimensionId(1),
                category_id: Some(ClassificationCategoryId(1)),
            },
        ]))
        .unwrap();
}

fn level(deflection: f64, max_tetrahedra: u32, volume_error: f64) -> ExactVolumeMeshWireOptions {
    ExactVolumeMeshWireOptions {
        surface_deflection_mm: deflection,
        angular_deflection_rad: deflection,
        max_tetrahedra,
        max_relative_volume_error: volume_error,
        min_tetrahedron_quality: 1.0e-6,
    }
}

/// The bottom and top faces by the height of their centroids.
fn end_faces(snapshot: &Snapshot) -> (u32, u32) {
    let graph = ExactBRepGraph::from_snapshot(snapshot, DefinitionId(1), FeatureId(2)).unwrap();
    let mut worker = ExactWorkerSupervisor::spawn(worker_path()).unwrap();
    let package = worker
        .evaluate_exact_brep_graph_volume_mesh(&graph, level(0.3, 512, 0.05))
        .unwrap();
    let faces = &package.source.face_evidence;
    let bottom = faces
        .iter()
        .min_by(|left, right| left.centroid_mm[2].total_cmp(&right.centroid_mm[2]))
        .unwrap();
    let top = faces
        .iter()
        .max_by(|left, right| left.centroid_mm[2].total_cmp(&right.centroid_mm[2]))
        .unwrap();
    (bottom.face_ordinal, top.face_ordinal)
}

fn request(bottom: u32, top: u32) -> ThermalStudyRequest {
    ThermalStudyRequest {
        definition_id: DefinitionId(1),
        feature_id: FeatureId(2),
        case_id: "pin-between-two-temperatures".into(),
        instance_path: InstancePath::root(OccurrenceId(1)),
        held_faces: vec![
            ExactThermalFaceTemperature {
                face_ordinal: bottom,
                temperature_c: 20.0,
            },
            ExactThermalFaceTemperature {
                face_ordinal: top,
                temperature_c: 120.0,
            },
        ],
        flux_faces: Vec::new(),
        convection_faces: Vec::new(),
        mesh_levels: vec![level(0.3, 512, 0.05), level(0.12, 1_024, 0.02)],
        solve_settings: ThermalSolveSettings::default(),
        convergence_relative_tolerance: 0.1,
    }
}

#[test]
fn a_steel_pin_between_two_temperatures_conducts_k_a_delta_t_over_h() {
    let _turn = crate::integration_support::file_turn();
    let mut document = cylinder();
    classify(&mut document, "steel");
    let snapshot = document.current();
    let (bottom, top) = end_faces(&snapshot);
    let rules = ValidationRules::library();
    let workflow = ThermalReviewWorkflow::new(Some(worker_path()));
    let cancelled = AtomicBool::new(false);

    assert_eq!(
        workflow.review(&snapshot, &request(bottom, top), rules, false, &cancelled),
        Err(ThermalReviewError::ConfirmationRequired)
    );
    let review = workflow
        .review(&snapshot, &request(bottom, top), rules, true, &cancelled)
        .unwrap();

    let steel = rules.materials["steel"]
        .thermal_conductivity
        .as_ref()
        .unwrap();
    assert_eq!(review.material.material, "steel");
    assert_eq!(review.material.conductivity_w_per_m_k, steel.w_m_k);
    assert!(review.material.citation.starts_with("EN 1993-1-2"));
    assert_eq!(review.units, "mm,°C,W");
    assert_eq!(review.levels.len(), 2);
    assert!(review.levels[1].element_count > review.levels[0].element_count);
    assert!(review.converged, "{review:#?}");
    assert_eq!(review.review_digest.len(), 64);
    // q = k ΔT / h through the section π r²; the meshed section is short
    // of the circle by at most the level's volume error.
    let exact_w = steel.w_m_k * 100.0 / (HEIGHT_MM / 1000.0)
        * std::f64::consts::PI
        * (RADIUS_MM / 1000.0).powi(2);
    for level in &review.levels {
        assert!((level.minimum_temperature_c - 20.0).abs() < 1e-9);
        assert!((level.maximum_temperature_c - 120.0).abs() < 1e-9);
        assert!(level.heat_balance_w.abs() < 1e-9 * exact_w);
        let heat_w = level.heat_into_held_faces_w;
        assert!(
            heat_w <= exact_w * (1.0 + 1e-9)
                && heat_w >= exact_w * (1.0 - level.relative_volume_error - 1e-9),
            "{heat_w} W against {exact_w} W, volume error {}",
            level.relative_volume_error
        );
    }
}

#[test]
fn a_part_whose_material_has_no_conductivity_is_refused_by_material() {
    let _turn = crate::integration_support::file_turn();
    let rules = ValidationRules::library();
    let workflow = ThermalReviewWorkflow::new(Some(worker_path()));
    let cancelled = AtomicBool::new(false);

    // Unclassified: the rules' default material, which names none.
    let document = cylinder();
    assert_eq!(
        workflow.review(&document.current(), &request(0, 1), rules, true, &cancelled),
        Err(ThermalReviewError::NoConductivity {
            material: rules.default_material.clone()
        })
    );

    let mut document = cylinder();
    classify(&mut document, "unobtainium");
    let refused = workflow
        .review(&document.current(), &request(0, 1), rules, true, &cancelled)
        .unwrap_err();
    assert_eq!(
        refused,
        ThermalReviewError::NoConductivity {
            material: "unobtainium".into()
        }
    );
    assert!(refused.to_string().contains("thermal_conductivity"));
}
