use std::fs;
use std::path::PathBuf;

use ketchup_core::assembly_joint::{
    AssemblyJoint, AssemblyJointAxis, AssemblyJointId, AssemblyJointKind, AssemblyJointLimits,
    AssemblyMotionDriver, AssemblyMotionStudy,
};
use ketchup_core::document::{CanonicalCommand, CommandBatch, OccurrenceId, Transform};
use ketchup_core::graph::sha256_hex;
use ketchup_core::import::{
    ImportLengthUnit, ImportOutputRef, StepImportEvidence, plan_step_import,
};
use ketchup_core::persistence::{self, ContainerData};
use ketchup_core::reference_examples::{
    HETTICH_EXAMPLE_BOTTOM, HETTICH_EXAMPLE_DRAWER, HETTICH_EXAMPLE_DRAWER_JOINT,
    HETTICH_EXAMPLE_LEFT_RUNNER, HETTICH_EXAMPLE_MOTION_STUDY, HETTICH_EXAMPLE_RIGHT_RUNNER,
    hettich_quadro_v6_drawer_example,
};
use ketchup_exact::{ExactBackend, ExactOpOutput};

const SOURCE_SHA256: &str = "10d92e2e8525cf328e0f09461051a37d388d92fa5af5d6e12e11369737c46238";
const LEFT_CABINET_INNER_Y_MM: f64 = 38.0;
const RIGHT_CABINET_INNER_Y_MM: f64 = 600.0;
const MOUNTING_CONTACT_TOLERANCE_MM: f64 = 1.0e-6;

#[derive(Clone, Copy)]
enum RunnerRole {
    Cabinet,
    Intermediate,
    Drawer,
}

#[derive(Clone, Copy)]
struct RunnerPart {
    name: &'static str,
    role: RunnerRole,
}

const RUNNER_PARTS: [RunnerPart; 6] = [
    RunnerPart {
        name: "Hettich right cabinet rail 9117663_4",
        role: RunnerRole::Cabinet,
    },
    RunnerPart {
        name: "Hettich left cabinet rail 9117663_3",
        role: RunnerRole::Cabinet,
    },
    RunnerPart {
        name: "Hettich left intermediate rail 9117663_1",
        role: RunnerRole::Intermediate,
    },
    RunnerPart {
        name: "Hettich left drawer rail 9117663_5",
        role: RunnerRole::Drawer,
    },
    RunnerPart {
        name: "Hettich right intermediate rail 9117663_2",
        role: RunnerRole::Intermediate,
    },
    RunnerPart {
        name: "Hettich right drawer rail 9117663_5",
        role: RunnerRole::Drawer,
    },
];

fn import_evidence(source_sha256: &str, output: &ExactOpOutput) -> StepImportEvidence {
    let topology = &output.body.topology;
    let signature = format!(
        "{source_sha256}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        output.backend_fingerprint,
        output.tolerance_report.profile,
        topology.vertex_count,
        topology.edge_count,
        topology.face_count,
        topology.shell_count,
        topology.solid_count,
        topology.volume_mm3.to_bits(),
        topology.bounds_mm.min.x.to_bits(),
        topology.bounds_mm.min.y.to_bits(),
        topology.bounds_mm.min.z.to_bits(),
        topology.bounds_mm.max.x.to_bits(),
        topology.bounds_mm.max.y.to_bits(),
        topology.bounds_mm.max.z.to_bits(),
    );
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in signature.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    StepImportEvidence {
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: format!("fnv1a64:{hash:016x}"),
        solid_count: topology.solid_count,
        topology_counts: [
            topology.vertex_count,
            topology.edge_count,
            topology.face_count,
            topology.shell_count,
            topology.solid_count,
        ],
        volume_mm3: topology.volume_mm3,
        bounds_mm: [
            [
                topology.bounds_mm.min.x,
                topology.bounds_mm.min.y,
                topology.bounds_mm.min.z,
            ],
            [
                topology.bounds_mm.max.x,
                topology.bounds_mm.max.y,
                topology.bounds_mm.max.z,
            ],
        ],
        backend: output.backend_fingerprint.to_owned(),
        tolerance: output.tolerance_report.profile.to_owned(),
    }
}

fn cabinet_mounting_plane_x(output: &ExactOpOutput, expected_outer_x_mm: f64) -> f64 {
    let face = output
        .body
        .topology
        .faces
        .iter()
        .filter(|face| {
            face.surface_kind == "plane"
                && face.area_mm2 > 13_000.0
                && (face.bounds_mm.max.x - face.bounds_mm.min.x).abs()
                    <= MOUNTING_CONTACT_TOLERANCE_MM
        })
        .min_by(|left, right| {
            (left.centroid_mm.x - expected_outer_x_mm)
                .abs()
                .total_cmp(&(right.centroid_mm.x - expected_outer_x_mm).abs())
        })
        .expect("cabinet member must retain its large perforated mounting face");
    assert!(
        (face.centroid_mm.x - expected_outer_x_mm).abs() <= MOUNTING_CONTACT_TOLERANCE_MM,
        "cabinet mounting face is at {}, expected {expected_outer_x_mm}",
        face.centroid_mm.x
    );
    face.centroid_mm.x
}

fn mounted_world_y(transform: Transform, local_x_mm: f64) -> f64 {
    let matrix = transform.matrix();
    matrix[4] * local_x_mm + matrix[7]
}

fn mounted_transform(open_offset_mm: f64) -> Transform {
    Transform::from_matrix([
        0.0,
        1.0,
        0.0,
        31.0 - open_offset_mm,
        -1.0,
        0.0,
        0.0,
        600.0,
        0.0,
        0.0,
        1.0,
        140.75,
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .unwrap()
}

fn main() {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("examples/hettich-quadro-v6-drawer.ketchup"));
    let source_path = std::env::args_os()
        .nth(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("work/hettich-quadro-v6-9129757/9129757.stp"));
    let source = fs::read(&source_path).unwrap();
    assert_eq!(sha256_hex(&source), SOURCE_SHA256);

    let backend = ExactBackend::new();
    let extracted_directory = tempfile::tempdir().unwrap();
    let mut document = hettich_quadro_v6_drawer_example();
    let mut container = ContainerData::default();
    assert_eq!(
        container.insert_import_blob(source.clone()).unwrap(),
        SOURCE_SHA256
    );
    let mut imported_occurrences = Vec::new();
    let mut cabinet_mounting_planes_x = Vec::new();

    for (ordinal, part) in RUNNER_PARTS.iter().enumerate() {
        let extracted = backend
            .import_step_solid(source_path.to_str().unwrap(), ordinal as u32)
            .unwrap();
        cabinet_mounting_planes_x.push(match part.role {
            RunnerRole::Cabinet if ordinal == 0 => Some(cabinet_mounting_plane_x(&extracted, 0.0)),
            RunnerRole::Cabinet if ordinal == 1 => {
                Some(cabinet_mounting_plane_x(&extracted, 562.0))
            }
            _ => None,
        });
        let extracted_path = extracted_directory
            .path()
            .join(format!("9129757-solid-{ordinal}.step"));
        backend
            .export_step(&extracted.body, extracted_path.to_str().unwrap())
            .unwrap();
        let extracted_source = fs::read(&extracted_path).unwrap();
        let verified = backend
            .import_step(extracted_path.to_str().unwrap())
            .unwrap();
        let source_name = format!("{}.step", part.name);
        let extracted_sha256 = sha256_hex(&extracted_source);
        let import = plan_step_import(
            &document.current(),
            &extracted_source,
            &source_name,
            &import_evidence(&extracted_sha256, &verified),
        )
        .unwrap();
        document.apply_batch(&import).unwrap();
        let occurrence = document
            .current()
            .import_receipts()
            .flat_map(|receipt| receipt.outputs())
            .filter_map(|output| match output {
                ImportOutputRef::Occurrence(id) => Some(*id),
                _ => None,
            })
            .max()
            .unwrap();
        imported_occurrences.push(occurrence);
        container.insert_import_blob(extracted_source).unwrap();
    }
    assert_eq!(
        imported_occurrences,
        (12..=17).map(OccurrenceId).collect::<Vec<_>>()
    );

    let mut commands = vec![
        CanonicalCommand::SetOccurrenceVisibility {
            id: HETTICH_EXAMPLE_LEFT_RUNNER,
            visible: false,
        },
        CanonicalCommand::SetOccurrenceVisibility {
            id: HETTICH_EXAMPLE_RIGHT_RUNNER,
            visible: false,
        },
    ];
    let intermediate_axis = AssemblyJointAxis::new([0.0, -1.0, 0.0], [0.0, 0.0, 0.0]);
    for (ordinal, (part, occurrence)) in RUNNER_PARTS
        .iter()
        .zip(imported_occurrences.iter().copied())
        .enumerate()
    {
        let open_offset_mm = match part.role {
            RunnerRole::Cabinet => 0.0,
            RunnerRole::Intermediate => 225.0,
            RunnerRole::Drawer => 450.0,
        };
        let transform = mounted_transform(open_offset_mm);
        if let Some(mounting_plane_x_mm) = cabinet_mounting_planes_x[ordinal] {
            let expected_wall_y_mm = if ordinal == 0 {
                RIGHT_CABINET_INNER_Y_MM
            } else {
                LEFT_CABINET_INNER_Y_MM
            };
            let actual_mounting_y_mm = mounted_world_y(transform, mounting_plane_x_mm);
            assert!(
                (actual_mounting_y_mm - expected_wall_y_mm).abs() <= MOUNTING_CONTACT_TOLERANCE_MM,
                "{} floats {} mm away from its cabinet wall",
                part.name,
                (actual_mounting_y_mm - expected_wall_y_mm).abs()
            );
        }
        commands.push(CanonicalCommand::SetOccurrenceTransform {
            id: occurrence,
            transform,
        });
        let kind = match part.role {
            RunnerRole::Cabinet | RunnerRole::Drawer => AssemblyJointKind::Fixed,
            RunnerRole::Intermediate => AssemblyJointKind::Prismatic {
                axis: intermediate_axis,
                limits: Some(AssemblyJointLimits::new(0.0, 225.0)),
                position_mm: 225.0,
            },
        };
        let parent = match part.role {
            RunnerRole::Cabinet => HETTICH_EXAMPLE_BOTTOM,
            RunnerRole::Drawer => HETTICH_EXAMPLE_DRAWER,
            RunnerRole::Intermediate if ordinal == 2 => imported_occurrences[1],
            RunnerRole::Intermediate => imported_occurrences[0],
        };
        commands.push(CanonicalCommand::CreateAssemblyJoint(AssemblyJoint::new(
            AssemblyJointId(111 + ordinal as u64),
            parent,
            occurrence,
            kind,
        )));
    }
    commands.push(CanonicalCommand::UpdateAssemblyMotionStudy(
        AssemblyMotionStudy::new(
            HETTICH_EXAMPLE_MOTION_STUDY,
            "Close articulated Hettich Quadro V6 drawer",
            vec![
                AssemblyMotionDriver::new(HETTICH_EXAMPLE_DRAWER_JOINT, 0.0),
                AssemblyMotionDriver::new(AssemblyJointId(113), 0.0),
                AssemblyMotionDriver::new(AssemblyJointId(115), 0.0),
            ],
        ),
    ));
    document.apply_batch(&CommandBatch::new(commands)).unwrap();

    persistence::save_atomic_with_container(&output, &document.current(), &container).unwrap();
    println!("{}", output.display());
}
