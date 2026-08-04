#![forbid(unsafe_code)]

use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    InstancePath, OccurrenceId, Transform,
};
use ketchup_interaction::projection::CanonicalInteractionProjection;
use ketchup_interaction::{
    ElementId, InteractionScene, LocaleCatalog, PreviewSession, Ray, SelectionFilter,
    SmartPushPullOutcome, SnapKind, Vec3, plan_smart_push_pull,
};
use ketchup_scheduler::ExactWorkerClient;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

const NODE: FeatureId = FeatureId(2);
const EDIT_WARMUP: usize = 100;
const EDIT_SAMPLES: usize = 1_000;
const PICK_WARMUP: usize = 200;
const PICK_SAMPLES: usize = 2_000;
const LONG_SAMPLES: usize = 100;
const OCCURRENCES: usize = 10_000;

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("Gate C measurements require a release build");
        std::process::exit(2);
    }
    let arguments = std::env::args().collect::<Vec<_>>();
    assert_eq!(
        arguments.len(),
        6,
        "usage: ketchup-gate-c-core <profile-id> <series> <r0-lock-sha256> <worker-exe> <output-json>"
    );
    let profile_id = &arguments[1];
    assert!(matches!(profile_id.as_str(), "HP-DEV-01" | "HP-IGPU-01"));
    let series = arguments[2]
        .parse::<usize>()
        .expect("series must be a positive integer");
    assert!((1..=3).contains(&series));
    let lock_sha256 = &arguments[3];
    assert_eq!(lock_sha256.len(), 64, "R0 lock SHA-256 must be complete");
    let worker_path = Path::new(&arguments[4]);
    assert!(worker_path.is_file(), "exact worker executable is missing");
    let output_path = PathBuf::from(&arguments[5]);

    let environment = environment_fingerprint();
    let mut worker = ExactWorkerClient::spawn(worker_path).expect("exact worker must start");
    worker
        .ping()
        .expect("exact worker must answer before measurement");
    let (edit_ms, digest_matches, result_fingerprint) = measure_edit(&mut worker);
    drop(worker);
    let scene = pick_scene();
    let (pick_ms, wrong_identities, class_counts) = measure_pick(&scene);
    let (navigation_block_ms, cancellation_ms, committed_data_loss) =
        measure_long(worker_path, &scene);

    let metrics = Metrics {
        edit_p95_ms: percentile(&edit_ms, 95),
        digest_match_percent: digest_matches as f64 / EDIT_SAMPLES as f64 * 100.0,
        pick_p95_ms: percentile(&pick_ms, 95),
        wrong_identities,
        navigation_block_max_ms: navigation_block_ms
            .iter()
            .copied()
            .max_by(f64::total_cmp)
            .unwrap_or_default(),
        cancellation_p95_ms: percentile(&cancellation_ms, 95),
        committed_data_loss,
    };
    write_metrics(
        &output_path,
        profile_id,
        series,
        lock_sha256,
        &environment,
        &result_fingerprint,
        &metrics,
        class_counts,
        &edit_ms,
        &pick_ms,
        &navigation_block_ms,
        &cancellation_ms,
    );

    assert!(metrics.edit_p95_ms <= 100.0, "{metrics:?}");
    assert_eq!(metrics.digest_match_percent, 100.0, "{metrics:?}");
    assert!(metrics.pick_p95_ms <= 50.0, "{metrics:?}");
    assert_eq!(metrics.wrong_identities, 0, "{metrics:?}");
    assert!(metrics.navigation_block_max_ms <= 100.0, "{metrics:?}");
    assert!(metrics.cancellation_p95_ms <= 250.0, "{metrics:?}");
    assert_eq!(metrics.committed_data_loss, 0, "{metrics:?}");
    println!(
        "Gate C core series {series} PASS: edit p95 {:.4} ms, pick p95 {:.4} ms, cancellation p95 {:.4} ms",
        metrics.edit_p95_ms, metrics.pick_p95_ms, metrics.cancellation_p95_ms
    );
}

fn measure_edit(worker: &mut ExactWorkerClient) -> (Vec<f64>, usize, String) {
    for sample in 0..EDIT_WARMUP {
        worker
            .extrude_rectangle(target_height(sample))
            .expect("edit warmup must succeed");
    }
    let catalog = LocaleCatalog::english();
    let mut document = seed_document();
    let mut samples = Vec::with_capacity(EDIT_SAMPLES);
    let mut digest_matches = 0;
    let mut result_fingerprint = String::new();
    for sample in 0..EDIT_SAMPLES {
        let height = target_height(sample + 1);
        let height_text = format!("{height:.0}");
        let plan = match plan_smart_push_pull(&document, &[NODE], &height_text)
            .expect("baseline push/pull plan must be valid")
        {
            SmartPushPullOutcome::Ready(plan) => plan,
            SmartPushPullOutcome::NeedsChoice { .. } => {
                panic!("baseline source extrusion must be unambiguous")
            }
        };
        let preview_digest = plan.action_digest().command_digest.clone();
        let rendered = plan.action_digest().render(&catalog);
        assert!(rendered.contains(&height_text));
        let started = Instant::now();
        let exact = worker
            .extrude_rectangle(height)
            .expect("baseline exact edit must succeed");
        let committed = PreviewSession::new(sample as u64 + 1, plan)
            .confirm(&mut document)
            .expect("preview confirmation must commit atomically");
        samples.push(milliseconds(started.elapsed()));
        if committed.action_digest.command_digest == preview_digest
            && committed.revision.batch_digest() == preview_digest
        {
            digest_matches += 1;
        }
        assert_eq!(exact.volume_mm3.to_bits(), (6_000.0 * height).to_bits());
        result_fingerprint = exact.result_fingerprint;
    }
    (samples, digest_matches, result_fingerprint)
}

fn measure_pick(scene: &InteractionScene) -> (Vec<f64>, usize, [usize; 5]) {
    for sample in 0..PICK_WARMUP {
        let _ = execute_pick(scene, sample).expect("pick warmup must resolve");
    }
    let mut samples = Vec::with_capacity(PICK_SAMPLES);
    let mut wrong_identities = 0;
    let mut class_counts = [0; 5];
    for sample in 0..PICK_SAMPLES {
        let started = Instant::now();
        let correct = execute_pick(scene, sample).unwrap_or(false);
        samples.push(milliseconds(started.elapsed()));
        if !correct {
            wrong_identities += 1;
        }
        class_counts[sample % class_counts.len()] += 1;
    }
    (samples, wrong_identities, class_counts)
}

fn execute_pick(scene: &InteractionScene, sample: usize) -> Option<bool> {
    let class = sample % 5;
    if class == 4 {
        let ray = Ray::new(Vec3::new(50.0, 30.0, 100.0), Vec3::new(0.0, 0.0, -1.0)).ok()?;
        let result = scene.exact_pick(ray, 0.01)?;
        return Some(matches!(
            result.snap.reference.element,
            ElementId::Intersection {
                ref other_instance_path,
                ..
            } if *other_instance_path == InstancePath::root(OccurrenceId(2))
        ));
    }

    let grid_index = 2 + sample % (OCCURRENCES - 2);
    let occurrence_id = grid_index as u64 + 1;
    let origin = grid_origin(grid_index);
    let (point, filter) = match class {
        0 => (
            Vec3::new(origin.x + 50.0, origin.y + 30.0, 100.0),
            SelectionFilter::Face,
        ),
        1 => (
            Vec3::new(origin.x + 50.0, origin.y, 100.0),
            SelectionFilter::Edge,
        ),
        2 => (Vec3::new(origin.x, origin.y, 100.0), SelectionFilter::Point),
        3 => (
            Vec3::new(origin.x + 50.0, origin.y, 100.0),
            SelectionFilter::Face,
        ),
        _ => unreachable!(),
    };
    let ray = Ray::new(point, Vec3::new(0.0, 0.0, -1.0)).ok()?;
    let result = scene.exact_pick_filtered(ray, 0.01, filter)?;
    let class_correct = match class {
        0 => matches!(result.primary.reference.element, ElementId::Face { .. }),
        1 => matches!(result.primary.reference.element, ElementId::Edge(_)),
        2 => matches!(result.primary.reference.element, ElementId::Endpoint(_)),
        3 => result.snap.kind == SnapKind::Midpoint,
        _ => false,
    };
    Some(
        result.primary.reference.instance_path == InstancePath::root(OccurrenceId(occurrence_id))
            && class_correct,
    )
}

fn measure_long(worker_path: &Path, scene: &InteractionScene) -> (Vec<f64>, Vec<f64>, usize) {
    let document = seed_document();
    let committed_digest = document.current().canonical_digest();
    let mut navigation_block_ms = Vec::with_capacity(LONG_SAMPLES);
    let mut cancellation_ms = Vec::with_capacity(LONG_SAMPLES);
    let mut committed_data_loss = 0;
    for sample in 0..LONG_SAMPLES {
        let mut worker = ExactWorkerClient::spawn(worker_path).expect("long worker must start");
        worker
            .begin_killable_job(Duration::from_secs(2))
            .expect("long job must start");
        let started = Instant::now();
        assert!(execute_pick(scene, sample).unwrap_or(false));
        navigation_block_ms.push(milliseconds(started.elapsed()));
        std::thread::sleep(Duration::from_millis(2));
        cancellation_ms.push(milliseconds(
            worker.cancel().expect("long job must cancel cleanly"),
        ));
        if document.current().canonical_digest() != committed_digest {
            committed_data_loss += 1;
        }
    }
    (navigation_block_ms, cancellation_ms, committed_data_loss)
}

fn pick_scene() -> InteractionScene {
    let definition_id = DefinitionId(1);
    let profile_id = FeatureId(1);
    let extrusion_id = FeatureId(2);
    let mut commands = vec![
        CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: "Gate C box".to_owned(),
        },
        CanonicalCommand::CreateFeature {
            id: profile_id,
            definition_id,
            name: "Gate C profile".to_owned(),
            kind: FeatureKind::Profile {
                points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]],
            },
        },
        CanonicalCommand::CreateFeature {
            id: extrusion_id,
            definition_id,
            name: "Gate C extrusion".to_owned(),
            kind: FeatureKind::Extrusion {
                profile: profile_id,
                height: Dimension::from_decimal("20").unwrap(),
            },
        },
    ];
    commands.extend((0..OCCURRENCES).map(|index| {
        let origin = match index {
            0 => Vec3::new(0.0, 30.0, 0.0),
            1 => Vec3::new(50.0, 0.0, 0.0),
            _ => grid_origin(index),
        };
        CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(index as u64 + 1),
            definition_id,
            name: format!("Gate C occurrence {}", index + 1),
            transform: Transform::from_translation(origin.x, origin.y, origin.z).unwrap(),
            parent: None,
            tag: None,
            visible: true,
        }
    }));
    let mut document = DocumentStore::new();
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let scene = CanonicalInteractionProjection::from_snapshot(&document.current())
        .scene()
        .unwrap();
    assert_eq!(scene.occurrence_count(), OCCURRENCES);
    assert_eq!(scene.authoritative_geometry_count(), 1);
    scene
}

fn grid_origin(index: usize) -> Vec3 {
    let adjusted = index - 2;
    Vec3::new(
        1_000.0 + (adjusted % 100) as f64 * 125.0,
        1_000.0 + (adjusted / 100) as f64 * 85.0,
        0.0,
    )
}

fn seed_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Gate C".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [100.0, 0.0], [100.0, 60.0], [0.0, 60.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: NODE,
                definition_id: DefinitionId(1),
                name: "Extrude-1".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::from_decimal("20").unwrap(),
                },
            },
        ]))
        .unwrap();
    document
}

fn target_height(sample: usize) -> f64 {
    if sample.is_multiple_of(2) { 20.0 } else { 35.0 }
}

fn percentile(samples: &[f64], percentile: usize) -> f64 {
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    let index = (ordered.len() * percentile).div_ceil(100) - 1;
    ordered[index]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn environment_fingerprint() -> String {
    let script = concat!(
        "$cpu=Get-CimInstance Win32_Processor|Select-Object -First 1 Name,NumberOfCores,NumberOfLogicalProcessors;",
        "$gpu=Get-CimInstance Win32_VideoController|Select-Object Name,DriverVersion,AdapterRAM;",
        "$os=Get-CimInstance Win32_OperatingSystem|Select-Object Caption,Version,BuildNumber,TotalVisibleMemorySize;",
        "[pscustomobject]@{cpu=$cpu;gpu=@($gpu);os=$os}|ConvertTo-Json -Compress -Depth 4"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .expect("PowerShell must capture the Windows environment");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("environment JSON must be UTF-8")
        .trim()
        .to_owned()
}

#[derive(Debug)]
struct Metrics {
    edit_p95_ms: f64,
    digest_match_percent: f64,
    pick_p95_ms: f64,
    wrong_identities: usize,
    navigation_block_max_ms: f64,
    cancellation_p95_ms: f64,
    committed_data_loss: usize,
}

#[allow(clippy::too_many_arguments)]
fn write_metrics(
    output_path: &Path,
    profile_id: &str,
    series: usize,
    lock_sha256: &str,
    environment: &str,
    result_fingerprint: &str,
    metrics: &Metrics,
    class_counts: [usize; 5],
    edit_ms: &[f64],
    pick_ms: &[f64],
    navigation_block_ms: &[f64],
    cancellation_ms: &[f64],
) {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).expect("Gate C artifact directory must be writable");
    }
    let mut json = String::new();
    writeln!(json, "{{").unwrap();
    writeln!(json, "  \"schema_version\": 1,").unwrap();
    writeln!(json, "  \"profile_id\": \"{profile_id}\",").unwrap();
    writeln!(json, "  \"series\": {series},").unwrap();
    writeln!(json, "  \"r0_lock_sha256\": \"{lock_sha256}\",").unwrap();
    writeln!(json, "  \"environment\": {environment},").unwrap();
    writeln!(json, "  \"result_fingerprint\": \"{result_fingerprint}\",").unwrap();
    writeln!(json, "  \"occurrences\": {OCCURRENCES},").unwrap();
    writeln!(json, "  \"edit_warmup\": {EDIT_WARMUP},").unwrap();
    writeln!(json, "  \"edit_samples\": {EDIT_SAMPLES},").unwrap();
    writeln!(json, "  \"pick_warmup\": {PICK_WARMUP},").unwrap();
    writeln!(json, "  \"pick_samples\": {PICK_SAMPLES},").unwrap();
    writeln!(json, "  \"long_samples\": {LONG_SAMPLES},").unwrap();
    writeln!(json, "  \"edit_p95_ms\": {:.6},", metrics.edit_p95_ms).unwrap();
    writeln!(
        json,
        "  \"action_digest_match_percent\": {:.6},",
        metrics.digest_match_percent
    )
    .unwrap();
    writeln!(json, "  \"pick_snap_p95_ms\": {:.6},", metrics.pick_p95_ms).unwrap();
    writeln!(
        json,
        "  \"wrong_identity_count\": {},",
        metrics.wrong_identities
    )
    .unwrap();
    writeln!(
        json,
        "  \"navigation_block_max_ms\": {:.6},",
        metrics.navigation_block_max_ms
    )
    .unwrap();
    writeln!(
        json,
        "  \"cancel_p95_ms\": {:.6},",
        metrics.cancellation_p95_ms
    )
    .unwrap();
    writeln!(
        json,
        "  \"committed_data_loss_count\": {},",
        metrics.committed_data_loss
    )
    .unwrap();
    writeln!(
        json,
        "  \"pick_class_counts\": {{\"face\": {}, \"edge\": {}, \"endpoint\": {}, \"midpoint\": {}, \"intersection\": {}}},",
        class_counts[0], class_counts[1], class_counts[2], class_counts[3], class_counts[4]
    )
    .unwrap();
    write_samples(&mut json, "edit_ms", edit_ms, true);
    write_samples(&mut json, "pick_snap_ms", pick_ms, true);
    write_samples(&mut json, "navigation_block_ms", navigation_block_ms, true);
    write_samples(&mut json, "cancellation_ms", cancellation_ms, false);
    writeln!(json, "}}").unwrap();
    std::fs::write(output_path, json).expect("Gate C raw metrics must be written");
}

fn write_samples(json: &mut String, name: &str, samples: &[f64], comma: bool) {
    write!(json, "  \"{name}\": [").unwrap();
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        write!(json, "{sample:.6}").unwrap();
    }
    writeln!(json, "]{}", if comma { "," } else { "" }).unwrap();
}
