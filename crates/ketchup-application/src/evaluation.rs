use ketchup_core::document::{
    DefinitionId, DocumentId, DocumentStore, FeatureId, FeatureKind, Snapshot,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepGraphError, ExactBRepOperation};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFeatureChainRequest, ExactResultRegistry, ImportedExactPackage,
};
#[cfg(feature = "named-product-fixtures")]
use ketchup_core::exact_revolve::ExactRevolveRequest;
use ketchup_core::graph::sha256_bytes;
use ketchup_core::import::{
    ImportFormat, ImportUnitAuthority, STEP_PARSER_ID, STEP_PARSER_VERSION, StepImportEvidence,
};
use ketchup_core::persistence::ContainerData;
use ketchup_core::sketch::{WorkplaneSpec, WorkplaneSupport};
use ketchup_scheduler::{
    ExactWorkerSupervisor, MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES,
    MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES,
};
use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
};
use std::time::Duration;
enum ExactEvaluationRequest {
    Graph {
        graph: Box<ExactBRepGraph>,
        imported_sources: Vec<Vec<u8>>,
    },
    Rectangle {
        request: Box<ExactFeatureChainRequest>,
        topology: Option<Box<ExactBRepGraph>>,
    },
    #[cfg(feature = "named-product-fixtures")]
    Revolve(Box<ExactRevolveRequest>),
    Imported(DefinitionId, Vec<u8>),
}

type PreparedRequests = (
    Vec<(ProducerKey, ExactEvaluationRequest)>,
    Vec<ProducerCoverage>,
);

fn prepare_requests(
    snapshot: &Snapshot,
    container_data: &ContainerData,
    exact_results: &ExactResultRegistry,
    topology_results: &ExactResultRegistry,
) -> Result<PreparedRequests, String> {
    let feature_graph = snapshot
        .feature_dependency_graph()
        .map_err(|error| error.to_string())?;
    let referenced_producers = snapshot
        .features()
        .filter_map(|feature| match feature.kind() {
            FeatureKind::Workplane(WorkplaneSpec {
                support: WorkplaneSupport::PlanarFace { reference, .. },
                ..
            }) => Some(reference.producer_feature_id),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let producers = snapshot
        .scene_query()
        .into_iter()
        .map(|occurrence| occurrence.definition_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .flat_map(|definition_id| {
            let Some(definition) = snapshot.definition(definition_id) else {
                return Vec::new();
            };
            definition
                .feature_ids()
                .iter()
                .copied()
                .filter(|feature_id| {
                    snapshot.feature(*feature_id).is_some_and(|feature| {
                        if !feature.kind().produces_body() {
                            return false;
                        }
                        if referenced_producers.contains(feature_id) {
                            return true;
                        }
                        let Some(body_id) = definition
                            .feature_body_ownership(*feature_id)
                            .and_then(|ownership| ownership.output_body_id())
                        else {
                            return false;
                        };
                        let suppressed = snapshot.suppressed_feature_ids(definition_id, body_id);
                        if suppressed.is_some_and(|ids| ids.contains(feature_id)) {
                            return false;
                        }
                        feature_graph
                            .dependents(*feature_id)
                            .is_some_and(|dependents| {
                                dependents.iter().all(|dependent| {
                                    suppressed.is_some_and(|ids| ids.contains(dependent))
                                        || snapshot.feature(*dependent).is_none_or(|feature| {
                                            !feature.kind().produces_body()
                                                || definition
                                                    .feature_body_ownership(*dependent)
                                                    .and_then(|ownership| {
                                                        ownership.output_body_id()
                                                    })
                                                    != Some(body_id)
                                        })
                                })
                            })
                    })
                })
                .map(move |feature_id| (definition_id, feature_id))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut requests = Vec::new();
    let mut coverage = Vec::new();
    for (definition_id, feature_id) in producers {
        let key = ProducerKey {
            definition_id,
            feature_id,
        };
        let current = |registry: &ExactResultRegistry| {
            registry.values().any(|package| {
                package.definition_id() == definition_id
                    && package.producer_feature_id() == feature_id
                    && package.is_current(snapshot)
            })
        };
        if current(exact_results) {
            coverage.push(ProducerCoverage {
                key,
                render: EvidenceStatus::Current,
                topology: if current(topology_results) {
                    EvidenceStatus::Current
                } else {
                    EvidenceStatus::not_evaluated("topology not provided by this request")
                },
            });
            continue;
        }
        let compiled = (|| -> Result<Option<(DefinitionId, ExactEvaluationRequest)>, String> {
            if let Ok(request) = ExactFeatureChainRequest::from_snapshot_for_producer(
                snapshot,
                definition_id,
                feature_id,
            ) {
                let topology = snapshot
                    .feature(feature_id)
                    .filter(|feature| {
                        matches!(
                            feature.kind(),
                            FeatureKind::Extrusion { .. } | FeatureKind::Pad(_)
                        )
                    })
                    .and_then(|_| {
                        ExactBRepGraph::from_snapshot(snapshot, definition_id, feature_id).ok()
                    })
                    .map(Box::new);
                return Ok(Some((
                    definition_id,
                    ExactEvaluationRequest::Rectangle {
                        request: Box::new(request),
                        topology,
                    },
                )));
            }
            #[cfg(feature = "named-product-fixtures")]
            if let Ok(request) = ExactRevolveRequest::from_snapshot(snapshot, definition_id)
                && request.producer_feature_id() == feature_id
            {
                return Ok(Some((
                    definition_id,
                    ExactEvaluationRequest::Revolve(Box::new(request)),
                )));
            }
            let graph = if snapshot
                .feature(feature_id)
                .is_some_and(|feature| !matches!(feature.kind(), FeatureKind::ImportedExactBody(_)))
            {
                match ExactBRepGraph::from_snapshot(snapshot, definition_id, feature_id) {
                    Ok(graph) => Some(graph),
                    Err(
                        ExactBRepGraphError::UnsupportedFeature(_)
                        | ExactBRepGraphError::UnsupportedProfile(_),
                    ) => None,
                    Err(error) => {
                        eprintln!(
                            "exact B-Rep graph compilation rejected producer {}: {error}",
                            feature_id.0
                        );
                        return Err("unsupported or unavailable exact producer/source".to_owned());
                    }
                }
            } else {
                None
            };
            if let Some(graph) = graph {
                let mut imported_sources = Vec::new();
                let mut imported_hashes = Vec::new();
                let mut imported_source_bytes = 0_u64;
                for node in &graph.nodes {
                    let ExactBRepOperation::ImportedExact {
                        source_sha256,
                        source_byte_len,
                        ..
                    } = &node.operation
                    else {
                        continue;
                    };
                    if imported_hashes.contains(source_sha256) {
                        continue;
                    }
                    let Some(next_source_bytes) =
                        imported_source_bytes.checked_add(*source_byte_len)
                    else {
                        eprintln!(
                            "exact B-Rep graph producer {} exceeds the imported source byte envelope",
                            feature_id.0
                        );
                        return Err("unsupported or unavailable exact producer/source".to_owned());
                    };
                    if imported_hashes.len() >= MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES
                        || next_source_bytes > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES
                    {
                        eprintln!(
                            "exact B-Rep graph producer {} exceeds the imported source envelope",
                            feature_id.0
                        );
                        return Err("unsupported or unavailable exact producer/source".to_owned());
                    }
                    imported_source_bytes = next_source_bytes;
                    let hash = source_sha256
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    let Some(source) = container_data.blobs().get(&hash).cloned() else {
                        eprintln!(
                            "exact B-Rep graph producer {} is missing an imported source blob",
                            feature_id.0
                        );
                        return Err("unsupported or unavailable exact producer/source".to_owned());
                    };
                    if source.len() as u64 != *source_byte_len
                        || sha256_bytes(&source) != *source_sha256
                    {
                        eprintln!(
                            "exact B-Rep graph producer {} has a mismatched imported source blob",
                            feature_id.0
                        );
                        return Err("unsupported or unavailable exact producer/source".to_owned());
                    }
                    imported_hashes.push(*source_sha256);
                    imported_sources.push(source);
                }
                return Ok(Some((
                    definition_id,
                    ExactEvaluationRequest::Graph {
                        graph: Box::new(graph),
                        imported_sources,
                    },
                )));
            }
            let Some(feature) = snapshot.feature(feature_id) else {
                return Err("unsupported or unavailable exact producer/source".to_owned());
            };
            let FeatureKind::ImportedExactBody(spec) = feature.kind() else {
                return Err("unsupported or unavailable exact producer/source".to_owned());
            };
            let hash = spec
                .source_sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            let source = container_data
                .blobs()
                .get(&hash)
                .cloned()
                .ok_or_else(|| "imported STEP source blob is missing".to_owned())?;
            if source.len() as u64 != spec.source_byte_len
                || sha256_bytes(&source) != spec.source_sha256
            {
                return Err(
                    "imported STEP source blob does not match canonical identity".to_owned(),
                );
            }
            Ok(Some((
                definition_id,
                ExactEvaluationRequest::Imported(definition_id, source),
            )))
        })();
        match compiled {
            Ok(Some((_, request))) => {
                coverage.push(ProducerCoverage {
                    key,
                    render: EvidenceStatus::not_evaluated("pending"),
                    topology: EvidenceStatus::not_evaluated("pending"),
                });
                requests.push((key, request));
            }
            Ok(None) => coverage.push(ProducerCoverage {
                key,
                render: EvidenceStatus::not_evaluated("unsupported producer"),
                topology: EvidenceStatus::not_evaluated("unsupported producer"),
            }),
            Err(reason) => coverage.push(ProducerCoverage {
                key,
                render: EvidenceStatus::Failed {
                    reason: reason.clone(),
                },
                topology: EvidenceStatus::not_evaluated(&reason),
            }),
        }
    }
    Ok((requests, coverage))
}

pub fn exact_worker_candidates() -> Vec<PathBuf> {
    let executable_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let Some(current) = std::env::current_exe().ok() else {
        return Vec::new();
    };
    let Some(parent) = current.parent() else {
        return Vec::new();
    };
    let mut candidates = vec![parent.join(executable_name)];
    if let Some(grandparent) = parent.parent() {
        candidates.push(grandparent.join(executable_name));
    }
    candidates
}

#[path = "evaluation_task.rs"]
mod task;
pub use task::*;

/// Starts the same worker task used by desktop polling and headless waiting.
/// The callback is notification only; it cannot influence evaluation or publication.
pub fn start_exact_evaluation(
    snapshot: Snapshot,
    container_data: &ContainerData,
    render: &ExactResultRegistry,
    topology: &ExactResultRegistry,
    executable: Option<PathBuf>,
    completed: impl FnOnce() + Send + 'static,
) -> ExactEvaluationTask {
    let source = exact_source(&snapshot);
    let prepared = prepare_requests(&snapshot, container_data, render, topology);
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    let worker_source = source.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut report = EvaluationReport {
            source: worker_source.clone(),
            producers: Vec::new(),
            complete: false,
            topology_complete: false,
            not_evaluated: None,
        };
        let mut render_packages = Vec::new();
        let mut topology_packages = Vec::new();
        match prepared {
            Err(error) => report.not_evaluated = Some(error),
            Ok((requests, coverage)) => {
                report.producers = coverage;
                if !requests.is_empty() {
                    let worker = executable
                        .ok_or_else(|| "exact worker unavailable".to_owned())
                        .and_then(|path| {
                            ExactWorkerSupervisor::spawn_with_cancellation(path, &worker_cancelled)
                                .map_err(|error| error.to_string())
                        });
                    match worker {
                        Err(reason) => {
                            for (key, _) in &requests {
                                let entry = report
                                    .producers
                                    .iter_mut()
                                    .find(|entry| entry.key == *key)
                                    .expect("selected producer");
                                entry.render = EvidenceStatus::not_evaluated(&reason);
                                entry.topology = EvidenceStatus::not_evaluated(&reason);
                            }
                            report.not_evaluated = Some(reason);
                        }
                        Ok(mut worker) => {
                            for (key, request) in requests {
                                if worker_cancelled.load(Ordering::Acquire) {
                                    break;
                                }
                                let definition_id = key.definition_id;
                                let entry = report
                                    .producers
                                    .iter_mut()
                                    .find(|entry| entry.key == key)
                                    .expect("selected producer");
                                let mut topology_failure = None;
                                let evaluated =
    (|| -> Result<(ExactBodyPackage, Option<ExactBodyPackage>), String> {
        Ok(match request {
            ExactEvaluationRequest::Graph {
                graph,
                imported_sources,
            } => {
                let imported_sources = imported_sources
                    .iter()
                    .map(Vec::as_slice)
                    .collect::<Vec<_>>();
                let package = worker
                    .evaluate_exact_brep_graph_with_imported_sources_and_cancellation(
                        &graph,
                        &imported_sources,
                &worker_cancelled,
                    )
                    .map(ExactBodyPackage::Graph)
                    .map_err(|error| error.to_string())?;
                (package.clone(), Some(package))
            }
            ExactEvaluationRequest::Rectangle { request, topology } => {
                let package = worker
                    .evaluate_rectangle_with_cancellation(
                        &request,
                        &worker_cancelled,
                    )
                    .map(ExactBodyPackage::from)
                    .map_err(|error| error.to_string())?;
                let topology_package = topology.and_then(|graph| {
                    match worker.evaluate_exact_brep_graph_with_imported_sources_and_cancellation(&graph, &[], &worker_cancelled) {
                        Ok(package) => Some(ExactBodyPackage::Graph(package)),
                        Err(error) => {
                            eprintln!(
                                "exact topology evaluation rejected definition {}: {error}",
                                definition_id.0
                            );
                            topology_failure = Some(error.to_string());
                            None
                        }
                    }
                });
                (package, topology_package)
            }
            #[cfg(feature = "named-product-fixtures")]
            ExactEvaluationRequest::Revolve(request) => (
                worker
                    .evaluate_revolve_with_cancellation(
                        &request,
                        &worker_cancelled,
                    )
                    .map(ExactBodyPackage::from)
                    .map_err(|error| error.to_string())?,
                None,
            ),
            ExactEvaluationRequest::Imported(definition_id, source) => {
                let definition =
                    snapshot.definition(definition_id).ok_or_else(|| {
                        "imported STEP definition is unavailable".to_owned()
                    })?;
                let [feature_id] = definition.feature_ids() else {
                    return Err(
                        "imported STEP definition is not singular".to_owned()
                    );
                };
                let feature =
                    snapshot.feature(*feature_id).ok_or_else(|| {
                        "imported STEP feature is unavailable".to_owned()
                    })?;
                let FeatureKind::ImportedExactBody(spec) = feature.kind()
                else {
                    return Err(
                        "imported STEP canonical specification is unavailable"
                            .to_owned(),
                    );
                };
                let receipt =
                    snapshot.import_receipt(spec.import_id).ok_or_else(
                        || "imported STEP receipt is unavailable".to_owned(),
                    )?;
                if receipt.format() != ImportFormat::Step
                    || receipt.units().authority()
                        != ImportUnitAuthority::FileDeclared
                    || receipt.parser_id() != STEP_PARSER_ID
                    || receipt.parser_version() != STEP_PARSER_VERSION
                {
                    return Err(
                        "imported STEP receipt provenance is not authoritative"
                            .to_owned(),
                    );
                }
                let source_unit = receipt.units().source_unit();
                let mut expected = StepImportEvidence {
                    source_unit,
                    result_fingerprint: spec.result_fingerprint.clone(),
                    solid_count: spec.solid_count,
                    topology_counts: spec.topology_counts.unwrap_or([0; 5]),
                    volume_mm3: spec.volume_mm3,
                    bounds_mm: spec.bounds_mm,
                    backend: spec.backend.clone(),
                    tolerance: spec.tolerance.clone(),
                };
                let mut temporary = tempfile::Builder::new()
                    .prefix("ketchup-imported-step-")
                    .suffix(".step")
                    .tempfile()
                    .map_err(|error| error.to_string())?;
                temporary
                    .write_all(&source)
                    .and_then(|_| temporary.flush())
                    .map_err(|error| error.to_string())?;
                let source_sha256 = ketchup_core::graph::sha256_hex(&source);
                let actual = worker
                    .inspect_step_import_with_cancellation(
                        temporary.path(),
                        &source_sha256,
                        &worker_cancelled,
                    )
                    .map_err(|error| error.to_string())?;
                if spec.topology_counts.is_none() {
                    expected.topology_counts = actual.topology_counts;
                }
                if actual != expected {
                    return Err(format!(
                        "imported STEP worker evidence does not match canonical specification: expected={expected:?}, actual={actual:?}"
                    ));
                }
                let mesh_target = tempfile::Builder::new()
                    .prefix("ketchup-imported-step-mesh-")
                    .suffix(".bin")
                    .tempfile()
                    .map_err(|error| error.to_string())?;
                let mesh = worker
                    .tessellate_step_import_with_cancellation(
                        temporary.path(),
                        &source_sha256,
                        &spec.result_fingerprint,
                        mesh_target.path(),
                        &worker_cancelled,
                    )
                    .map_err(|error| error.to_string())?;
                let package = ImportedExactPackage::from_snapshot(
                    &snapshot,
                    definition_id,
                    source,
                    &mesh,
                )
                .map(ExactBodyPackage::Imported)
                .map_err(|error| error.to_string())?;
                (package.clone(), Some(package))
            }
        })
    })();
                                let (package, topology_package) = match evaluated {
                                    Ok(products) => products,
                                    Err(error) => {
                                        eprintln!(
                                            "exact evaluation rejected definition {}: {error}",
                                            definition_id.0
                                        );
                                        entry.render = EvidenceStatus::Failed { reason: error };
                                        entry.topology = EvidenceStatus::not_evaluated(
                                            "render evaluation failed",
                                        );
                                        continue;
                                    }
                                };

                                entry.render = EvidenceStatus::Evaluated;
                                entry.topology = if topology_package.is_some() {
                                    EvidenceStatus::Evaluated
                                } else if let Some(reason) = topology_failure {
                                    EvidenceStatus::Failed { reason }
                                } else {
                                    EvidenceStatus::not_evaluated(
                                        "topology not provided by this request",
                                    )
                                };
                                render_packages.push(Arc::new(package));
                                if let Some(package) = topology_package {
                                    topology_packages.push(Arc::new(package));
                                }
                            }
                        }
                    }
                }
            }
        }
        report.finish();
        let products = ExactEvaluationProducts {
            source: worker_source,
            render_packages,
            topology_packages,
            report,
        };
        if !worker_cancelled.load(Ordering::Acquire) && sender.send(Ok(products)).is_ok() {
            completed();
        }
    });
    ExactEvaluationTask {
        source,
        cancelled,
        receiver,
    }
}
