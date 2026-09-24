use ketchup_core::document::{
    DefinitionId, DocumentStore, FeatureId, FeatureKind, OccurrenceId, SceneOccurrence, Snapshot,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_core::exact_product::{
    ExactBodyPackage, ExactFeatureChainRequest, ExactProducerCompilation,
    ExactProducerEvidenceContext, ExactProducerPlan, ExactResultRegistry, ImportedExactPackage,
    exact_body_terminal_features,
};
use ketchup_core::graph::sha256_bytes;
use ketchup_core::import::{
    IGES_PARSER_ID, IGES_PARSER_VERSION, IGES_XDE_PARSER_VERSION, ImportFormat,
    ImportUnitAuthority, STEP_PARSER_ID, STEP_PARSER_VERSION, STEP_XDE_PARSER_VERSION,
    StepImportEvidence,
};
use ketchup_core::persistence::ContainerData;
use ketchup_core::sketch::{WorkplaneSpec, WorkplaneSupport};
use ketchup_scheduler::{
    MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES, MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
};
use std::time::{Duration, Instant};
enum ExactEvaluationRequest {
    Graph {
        graph: Box<ExactBRepGraph>,
        imported_sources: Vec<Vec<u8>>,
    },
    Topology {
        graph: Box<ExactBRepGraph>,
        imported_sources: Vec<Vec<u8>>,
    },
    Rectangle {
        request: Box<ExactFeatureChainRequest>,
        topology: Option<Box<ExactBRepGraph>>,
    },
    Imported(DefinitionId, Vec<u8>),
}

type PreparedRequests = (
    Vec<(ProducerKey, ExactEvaluationRequest)>,
    Vec<ProducerCoverage>,
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncrementalExactScope {
    pub producers: BTreeSet<ProducerKey>,
    pub collision_occurrences: BTreeSet<OccurrenceId>,
    pub changed_feature_count: usize,
    pub changed_scene_occurrence_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncrementalExactPlan {
    pub selection: ExactEvaluationSelection,
    pub collision_occurrences: BTreeSet<OccurrenceId>,
    pub baseline_reused: bool,
    pub fallback_reason: Option<String>,
    pub changed_feature_count: usize,
    pub changed_scene_occurrence_count: usize,
}

pub fn plan_incremental_exact_evaluation(
    before: &Snapshot,
    after: &Snapshot,
    full_baseline: Option<&ExactSource>,
) -> Result<IncrementalExactPlan, String> {
    let scope = plan_incremental_exact_scope(before, after)?;
    let expected_baseline = exact_source(before);
    if full_baseline != Some(&expected_baseline) {
        return Ok(IncrementalExactPlan {
            selection: ExactEvaluationSelection::Full,
            collision_occurrences: after
                .scene_query()
                .into_iter()
                .filter(|occurrence| occurrence.visible)
                .map(|occurrence| occurrence.instance_path.root_occurrence())
                .collect(),
            baseline_reused: false,
            fallback_reason: Some("missing or stale complete exact baseline".to_owned()),
            changed_feature_count: scope.changed_feature_count,
            changed_scene_occurrence_count: scope.changed_scene_occurrence_count,
        });
    }
    Ok(IncrementalExactPlan {
        selection: ExactEvaluationSelection::Scoped(scope.producers),
        collision_occurrences: scope.collision_occurrences,
        baseline_reused: true,
        fallback_reason: None,
        changed_feature_count: scope.changed_feature_count,
        changed_scene_occurrence_count: scope.changed_scene_occurrence_count,
    })
}

fn scene_geometry_matches(left: &SceneOccurrence, right: &SceneOccurrence) -> bool {
    left.definition_id == right.definition_id
        && left.transform == right.transform
        && left.parent == right.parent
        && left.local_parent == right.local_parent
        && left.visible == right.visible
}

/// Derives the exact producers and world-space collision roots affected by a
/// canonical snapshot change. Geometry dependencies are closed through the
/// feature DAG; scene changes are compared after nested/shared expansion.
pub fn plan_incremental_exact_scope(
    before: &Snapshot,
    after: &Snapshot,
) -> Result<IncrementalExactScope, String> {
    if before.document_id() != after.document_id() {
        return Err("incremental exact scope requires snapshots from one document".to_owned());
    }

    let before_features = before
        .features()
        .map(|feature| (feature.id(), feature))
        .collect::<BTreeMap<_, _>>();
    let after_features = after
        .features()
        .map(|feature| (feature.id(), feature))
        .collect::<BTreeMap<_, _>>();
    let feature_ids = before_features
        .keys()
        .chain(after_features.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let changed_features = feature_ids
        .into_iter()
        .filter(|id| before_features.get(id) != after_features.get(id))
        .collect::<BTreeSet<_>>();

    let definition_ids = before
        .definitions()
        .map(|definition| definition.id())
        .chain(after.definitions().map(|definition| definition.id()))
        .collect::<BTreeSet<_>>();
    let mut structurally_changed_definitions = BTreeSet::new();
    let mut after_terminals = BTreeMap::new();
    for definition_id in definition_ids {
        let before_definition_terminals = if before.definition(definition_id).is_some() {
            exact_body_terminal_features(before, definition_id).map_err(|error| {
                format!(
                    "invalid previous exact terminal set for definition {definition_id:?}: {error}"
                )
            })?
        } else {
            BTreeMap::new()
        };
        let after_definition_terminals = if after.definition(definition_id).is_some() {
            exact_body_terminal_features(after, definition_id).map_err(|error| {
                format!(
                    "invalid current exact terminal set for definition {definition_id:?}: {error}"
                )
            })?
        } else {
            BTreeMap::new()
        };
        if before_definition_terminals != after_definition_terminals {
            structurally_changed_definitions.insert(definition_id);
        }
        after_terminals.insert(definition_id, after_definition_terminals);
    }

    let mut geometry_definitions = structurally_changed_definitions.clone();
    for feature_id in &changed_features {
        if let Some(feature) = after
            .feature(*feature_id)
            .or_else(|| before.feature(*feature_id))
        {
            geometry_definitions.insert(feature.definition_id());
        }
    }
    let current_changed_features = changed_features
        .iter()
        .copied()
        .filter(|id| after.feature(*id).is_some());
    let dirty_features = after
        .feature_dependency_graph()
        .map_err(|error| error.to_string())?
        .dependent_closure(current_changed_features);

    let after_scene = after
        .scene_query()
        .into_iter()
        .map(|occurrence| (occurrence.instance_path.clone(), occurrence))
        .collect::<BTreeMap<_, _>>();
    let visible_definitions = after_scene
        .values()
        .filter(|occurrence| occurrence.visible)
        .map(|occurrence| occurrence.definition_id)
        .collect::<BTreeSet<_>>();
    let mut producers = BTreeSet::new();
    for (definition_id, terminals) in &after_terminals {
        if !visible_definitions.contains(definition_id) {
            continue;
        }
        for feature_id in terminals.values() {
            if structurally_changed_definitions.contains(definition_id)
                || dirty_features.contains(feature_id)
            {
                producers.insert(ProducerKey {
                    definition_id: *definition_id,
                    feature_id: *feature_id,
                });
            }
        }
    }

    let before_scene = before
        .scene_query()
        .into_iter()
        .map(|occurrence| (occurrence.instance_path.clone(), occurrence))
        .collect::<BTreeMap<_, _>>();
    let scene_paths = before_scene
        .keys()
        .chain(after_scene.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let changed_scene_paths = scene_paths
        .into_iter()
        .filter(
            |path| match (before_scene.get(path), after_scene.get(path)) {
                (Some(left), Some(right)) => !scene_geometry_matches(left, right),
                (None, None) => false,
                _ => true,
            },
        )
        .collect::<BTreeSet<_>>();
    let mut collision_occurrences = changed_scene_paths
        .iter()
        .filter_map(|path| {
            after_scene
                .get(path)
                .filter(|occurrence| occurrence.visible)
                .map(|occurrence| occurrence.instance_path.root_occurrence())
        })
        .collect::<BTreeSet<_>>();
    collision_occurrences.extend(
        after_scene
            .values()
            .filter(|occurrence| {
                occurrence.visible && geometry_definitions.contains(&occurrence.definition_id)
            })
            .map(|occurrence| occurrence.instance_path.root_occurrence()),
    );

    Ok(IncrementalExactScope {
        producers,
        collision_occurrences,
        changed_feature_count: changed_features.len(),
        changed_scene_occurrence_count: changed_scene_paths.len(),
    })
}

fn prepare_requests(
    snapshot: &Snapshot,
    container_data: &ContainerData,
    exact_results: &ExactResultRegistry,
    topology_results: &ExactResultRegistry,
    scope: Option<&BTreeSet<ProducerKey>>,
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
    let mut producers = snapshot
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
    let available = producers
        .iter()
        .map(|(definition_id, feature_id)| ProducerKey {
            definition_id: *definition_id,
            feature_id: *feature_id,
        })
        .collect::<BTreeSet<_>>();
    if let Some(scope) = scope {
        producers.retain(|(definition_id, feature_id)| {
            scope.contains(&ProducerKey {
                definition_id: *definition_id,
                feature_id: *feature_id,
            })
        });
    }

    let mut requests = Vec::new();
    let mut coverage = scope
        .into_iter()
        .flat_map(|scope| scope.difference(&available))
        .map(|key| ProducerCoverage {
            key: *key,
            render: EvidenceStatus::not_evaluated(
                "requested producer is not a visible terminal or referenced body producer",
            ),
            topology: EvidenceStatus::not_evaluated(
                "requested producer is not a visible terminal or referenced body producer",
            ),
        })
        .collect::<Vec<_>>();
    let evidence_context = ExactProducerEvidenceContext::from_snapshot(snapshot);
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
        let render_current = current(exact_results);
        if render_current && current(topology_results) {
            coverage.push(ProducerCoverage {
                key,
                render: EvidenceStatus::Current,
                topology: EvidenceStatus::Current,
            });
            continue;
        }
        let compiled = (|| -> Result<Option<(DefinitionId, ExactEvaluationRequest)>, String> {
            let producer = ExactProducerCompilation::from_snapshot(
                snapshot,
                &evidence_context,
                definition_id,
                feature_id,
            )
            .map_err(|error| error.to_string())?;
            let Some(plan) = producer.plan().map_err(|error| {
                eprintln!(
                    "exact producer compilation rejected producer {}: {error}",
                    feature_id.0
                );
                "unsupported or unavailable exact producer/source".to_owned()
            })?
            else {
                return Ok(None);
            };
            match plan {
                ExactProducerPlan::Rectangle { request, topology } => Ok(Some((
                    definition_id,
                    ExactEvaluationRequest::Rectangle { request, topology },
                ))),
                ExactProducerPlan::Revolve(_) => {
                    unreachable!("legacy revolve planning is disabled")
                }
                ExactProducerPlan::Graph(graph) => {
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
                            return Err(
                                "unsupported or unavailable exact producer/source".to_owned()
                            );
                        };
                        if imported_hashes.len() >= MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES
                            || next_source_bytes > MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCE_BYTES
                        {
                            eprintln!(
                                "exact B-Rep graph producer {} exceeds the imported source envelope",
                                feature_id.0
                            );
                            return Err(
                                "unsupported or unavailable exact producer/source".to_owned()
                            );
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
                            return Err(
                                "unsupported or unavailable exact producer/source".to_owned()
                            );
                        };
                        if source.len() as u64 != *source_byte_len
                            || sha256_bytes(&source) != *source_sha256
                        {
                            eprintln!(
                                "exact B-Rep graph producer {} has a mismatched imported source blob",
                                feature_id.0
                            );
                            return Err(
                                "unsupported or unavailable exact producer/source".to_owned()
                            );
                        }
                        imported_hashes.push(*source_sha256);
                        imported_sources.push(source);
                    }
                    Ok(Some((
                        definition_id,
                        ExactEvaluationRequest::Graph {
                            graph,
                            imported_sources,
                        },
                    )))
                }
                ExactProducerPlan::Imported(spec) => {
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
                            "imported STEP source blob does not match canonical identity"
                                .to_owned(),
                        );
                    }
                    Ok(Some((
                        definition_id,
                        ExactEvaluationRequest::Imported(definition_id, source),
                    )))
                }
            }
        })();
        let compiled = if render_current {
            match compiled {
                Ok(Some((
                    id,
                    ExactEvaluationRequest::Graph {
                        graph,
                        imported_sources,
                    },
                ))) => Ok(Some((
                    id,
                    ExactEvaluationRequest::Topology {
                        graph,
                        imported_sources,
                    },
                ))),
                Ok(Some((
                    id,
                    ExactEvaluationRequest::Rectangle {
                        topology: Some(graph),
                        ..
                    },
                ))) => Ok(Some((
                    id,
                    ExactEvaluationRequest::Topology {
                        graph,
                        imported_sources: Vec::new(),
                    },
                ))),
                Ok(Some((id, request @ ExactEvaluationRequest::Imported(..)))) => {
                    Ok(Some((id, request)))
                }
                other => {
                    coverage.push(ProducerCoverage {
                        key,
                        render: EvidenceStatus::Current,
                        topology: match other {
                            Err(reason) => EvidenceStatus::Failed { reason },
                            _ => EvidenceStatus::not_evaluated(
                                "topology not provided by this request",
                            ),
                        },
                    });
                    continue;
                }
            }
        } else {
            compiled
        };
        match compiled {
            Ok(Some((_, request))) => {
                coverage.push(ProducerCoverage {
                    key,
                    render: if render_current {
                        EvidenceStatus::Current
                    } else {
                        EvidenceStatus::not_evaluated("pending")
                    },
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
    start_exact_evaluation_with_cancellation(
        snapshot,
        container_data,
        render,
        topology,
        executable,
        Arc::new(AtomicBool::new(false)),
        completed,
    )
}

pub fn start_exact_evaluation_with_cancellation(
    snapshot: Snapshot,
    container_data: &ContainerData,
    render: &ExactResultRegistry,
    topology: &ExactResultRegistry,
    executable: Option<PathBuf>,
    cancelled: Arc<AtomicBool>,
    completed: impl FnOnce() + Send + 'static,
) -> ExactEvaluationTask {
    start_exact_evaluation_scoped_with_cancellation(
        snapshot,
        container_data,
        render,
        topology,
        executable,
        None,
        cancelled,
        completed,
    )
}

pub fn start_exact_evaluation_scoped(
    snapshot: Snapshot,
    container_data: &ContainerData,
    render: &ExactResultRegistry,
    topology: &ExactResultRegistry,
    executable: Option<PathBuf>,
    scope: Option<&BTreeSet<ProducerKey>>,
    completed: impl FnOnce() + Send + 'static,
) -> ExactEvaluationTask {
    start_exact_evaluation_scoped_with_cancellation(
        snapshot,
        container_data,
        render,
        topology,
        executable,
        scope,
        Arc::new(AtomicBool::new(false)),
        completed,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn start_exact_evaluation_scoped_with_cancellation(
    snapshot: Snapshot,
    container_data: &ContainerData,
    render: &ExactResultRegistry,
    topology: &ExactResultRegistry,
    executable: Option<PathBuf>,
    scope: Option<&BTreeSet<ProducerKey>>,
    cancelled: Arc<AtomicBool>,
    completed: impl FnOnce() + Send + 'static,
) -> ExactEvaluationTask {
    let source = exact_source(&snapshot);
    let selection = scope.map_or(ExactEvaluationSelection::Full, |scope| {
        ExactEvaluationSelection::Scoped(scope.clone())
    });
    let render = ExactResultRegistry::carried_forward(&snapshot, render);
    let topology = ExactResultRegistry::carried_forward(&snapshot, topology);
    let prepared = prepare_requests(&snapshot, container_data, &render, &topology, scope);
    let initial_active_producer = prepared
        .as_ref()
        .ok()
        .and_then(|(requests, _)| requests.first().map(|(key, _)| *key));
    let (total_producers, initial_completed_producers, reused_producers) = match &prepared {
        Ok((requests, coverage)) => (
            coverage.len(),
            coverage.len().saturating_sub(requests.len()),
            coverage
                .iter()
                .filter(|entry| matches!(&entry.render, EvidenceStatus::Current))
                .count(),
        ),
        Err(_) => (0, 0, 0),
    };
    let completed_producers = Arc::new(AtomicUsize::new(initial_completed_producers));
    let worker_completed_producers = Arc::clone(&completed_producers);
    let active_producer = Arc::new(Mutex::new(
        initial_active_producer.map(|key| (key, Instant::now())),
    ));
    let worker_active_producer = Arc::clone(&active_producer);
    let worker_cancelled = Arc::clone(&cancelled);
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = Arc::clone(&finished);
    let worker_source = source.clone();
    let worker_selection = selection.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut report = EvaluationReport {
            source: worker_source.clone(),
            selection: worker_selection,
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
                            crate::worker_pool::checkout(&path, &worker_cancelled)
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
                                if !entry.render.is_evaluated() {
                                    entry.render = EvidenceStatus::not_evaluated(&reason);
                                }
                                entry.topology = EvidenceStatus::Failed {
                                    reason: reason.clone(),
                                };
                            }
                            report.not_evaluated = Some(reason);
                            worker_completed_producers.store(total_producers, Ordering::Release);
                        }
                        Ok(mut worker) => {
                            let mut worker_healthy = true;
                            for (key, request) in requests {
                                if worker_cancelled.load(Ordering::Acquire) {
                                    break;
                                }
                                *worker_active_producer
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                                    Some((key, Instant::now()));
                                let definition_id = key.definition_id;
                                let entry = report
                                    .producers
                                    .iter_mut()
                                    .find(|entry| entry.key == key)
                                    .expect("selected producer");
                                let topology_only = entry.render.is_evaluated();
                                let mut topology_failure = None;
                                let evaluated =
    (|| -> Result<(ExactBodyPackage, Option<ExactBodyPackage>), String> {
        Ok(match request {
            ExactEvaluationRequest::Graph {
                graph,
                imported_sources,
            } | ExactEvaluationRequest::Topology { graph, imported_sources } => {
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
                let (format_name, parser_id, parser_version, suffix) = match receipt.format() {
                    ImportFormat::Step => (
                        "STEP",
                        STEP_PARSER_ID,
                        if spec.source_part_index.is_some() {
                            STEP_XDE_PARSER_VERSION
                        } else {
                            STEP_PARSER_VERSION
                        },
                        ".step",
                    ),
                    ImportFormat::Iges => (
                        "IGES",
                        IGES_PARSER_ID,
                        if spec.source_part_index.is_some() {
                            IGES_XDE_PARSER_VERSION
                        } else {
                            IGES_PARSER_VERSION
                        },
                        ".iges",
                    ),
                    _ => return Err("imported exact receipt format is unsupported".to_owned()),
                };
                if receipt.units().authority() != ImportUnitAuthority::FileDeclared
                    || receipt.parser_id() != parser_id
                    || receipt.parser_version() != parser_version
                {
                    return Err(format!(
                        "imported {format_name} receipt provenance is not authoritative"
                    ));
                }
                let source_unit = receipt.units().source_unit();
                let mut expected = StepImportEvidence {
                    source_sha256: spec.source_sha256,
                    source_byte_len: spec.source_byte_len,
                    source_unit,
                    result_fingerprint: spec.result_fingerprint.clone(),
                    body_kind: spec.body_kind,
                    solid_count: spec.solid_count,
                    topology_counts: spec.topology_counts.unwrap_or([0; 5]),
                    area_mm2: spec.area_mm2,
                    volume_mm3: spec.volume_mm3,
                    bounds_mm: spec.bounds_mm,
                    backend: spec.backend.clone(),
                    tolerance: spec.tolerance.clone(),
                };
                let mut temporary = tempfile::Builder::new()
                    .prefix("ketchup-imported-exact-")
                    .suffix(suffix)
                    .tempfile()
                    .map_err(|error| error.to_string())?;
                temporary
                    .write_all(&source)
                    .and_then(|_| temporary.flush())
                    .map_err(|error| error.to_string())?;
                let source_sha256 = ketchup_core::graph::sha256_hex(&source);
                let actual = match receipt.format() {
                    ImportFormat::Step => match spec.source_part_index {
                        Some(index) => worker.inspect_step_xde_part_with_cancellation(
                            temporary.path(),
                            &source_sha256,
                            index,
                            &worker_cancelled,
                        ),
                        None => worker.inspect_step_import_with_cancellation(
                            temporary.path(),
                            &source_sha256,
                            &worker_cancelled,
                        ),
                    },
                    ImportFormat::Iges => match spec.source_part_index {
                        Some(index) => worker.inspect_iges_xde_part_with_cancellation(
                            temporary.path(),
                            &source_sha256,
                            index,
                            &worker_cancelled,
                        ),
                        None => worker
                            .inspect_iges_import_with_cancellation(
                                temporary.path(),
                                &source_sha256,
                                &worker_cancelled,
                            )
                            .map(|evidence| StepImportEvidence {
                                source_sha256: evidence.source_sha256,
                                source_byte_len: evidence.source_byte_len,
                                source_unit: evidence.source_unit,
                                result_fingerprint: evidence.result_fingerprint,
                                body_kind: evidence.body_kind,
                                solid_count: evidence.solid_count,
                                topology_counts: evidence.topology_counts,
                                area_mm2: evidence.area_mm2,
                                volume_mm3: evidence.volume_mm3,
                                bounds_mm: evidence.bounds_mm,
                                backend: evidence.backend,
                                tolerance: evidence.tolerance,
                            }),
                    },
                    _ => unreachable!("receipt format was validated above"),
                }
                .map_err(|error| error.to_string())?;
                if spec.topology_counts.is_none() {
                    expected.topology_counts = actual.topology_counts;
                }
                if spec.schema != ketchup_core::document::IMPORTED_EXACT_BODY_SCHEMA_V3 {
                    expected.area_mm2 = actual.area_mm2;
                }
                if actual != expected {
                    return Err(format!(
                        "imported {format_name} worker evidence does not match canonical specification: expected={expected:?}, actual={actual:?}"
                    ));
                }
                let mesh_target = tempfile::Builder::new()
                    .prefix("ketchup-imported-step-mesh-")
                    .suffix(".bin")
                    .tempfile()
                    .map_err(|error| error.to_string())?;
                let mesh = match receipt.format() {
                    ImportFormat::Step => match spec.source_part_index {
                        Some(index) => worker.tessellate_step_xde_part_with_cancellation(
                            temporary.path(),
                            &source_sha256,
                            &spec.result_fingerprint,
                            mesh_target.path(),
                            index,
                            &worker_cancelled,
                        ),
                        None => worker.tessellate_step_import_with_cancellation(
                            temporary.path(),
                            &source_sha256,
                            &spec.result_fingerprint,
                            mesh_target.path(),
                            &worker_cancelled,
                        ),
                    },
                    ImportFormat::Iges => match spec.source_part_index {
                        Some(index) => worker.tessellate_iges_xde_part_with_cancellation(
                            temporary.path(),
                            &source_sha256,
                            &spec.result_fingerprint,
                            mesh_target.path(),
                            index,
                            &worker_cancelled,
                        ),
                        None => worker.tessellate_iges_import_with_cancellation(
                            temporary.path(),
                            &source_sha256,
                            &spec.result_fingerprint,
                            mesh_target.path(),
                            &worker_cancelled,
                        ),
                    },
                    _ => unreachable!("receipt format was validated above"),
                }
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
                                        worker_healthy = false;
                                        if topology_only {
                                            entry.topology =
                                                EvidenceStatus::Failed { reason: error };
                                        } else {
                                            entry.render = EvidenceStatus::Failed { reason: error };
                                            entry.topology = EvidenceStatus::not_evaluated(
                                                "render evaluation failed",
                                            );
                                        }
                                        worker_completed_producers.fetch_add(1, Ordering::AcqRel);
                                        continue;
                                    }
                                };

                                if !topology_only {
                                    entry.render = EvidenceStatus::Evaluated;
                                }
                                entry.topology = if topology_package.is_some() {
                                    EvidenceStatus::Evaluated
                                } else if let Some(reason) = topology_failure {
                                    EvidenceStatus::Failed { reason }
                                } else {
                                    EvidenceStatus::not_evaluated(
                                        "topology not provided by this request",
                                    )
                                };
                                if !topology_only {
                                    render_packages.push(Arc::new(package));
                                }
                                if let Some(package) = topology_package {
                                    topology_packages.push(Arc::new(package));
                                }
                                worker_completed_producers.fetch_add(1, Ordering::AcqRel);
                            }
                            if worker_healthy && !worker_cancelled.load(Ordering::Acquire) {
                                worker.release();
                            }
                        }
                    }
                }
            }
        }
        *worker_active_producer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        report.finish();
        let products = ExactEvaluationProducts {
            source: worker_source,
            render_packages,
            topology_packages,
            report,
        };
        worker_finished.store(true, Ordering::Release);
        if !worker_cancelled.load(Ordering::Acquire) && sender.send(Ok(products)).is_ok() {
            completed();
        }
    });
    ExactEvaluationTask {
        source,
        selection,
        cancelled,
        finished,
        receiver,
        total_producers,
        completed_producers,
        reused_producers,
        active_producer,
    }
}

#[cfg(test)]
mod incremental_scope_tests {
    use super::*;
    use ketchup_core::document::{
        CanonicalCommand, CommandBatch, Dimension, GroupId, OccurrenceId, Transform,
    };

    #[test]
    fn shared_definition_change_excludes_independent_producers_and_transform_is_world_only() {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Shared".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(1),
                    definition_id: DefinitionId(1),
                    name: "Profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: DefinitionId(1),
                    name: "Solid".into(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(1),
                        height: Dimension::new("10", 10.0).unwrap(),
                    },
                },
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Independent".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(2),
                    name: "Profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [5.0, 0.0], [5.0, 5.0], [0.0, 5.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: DefinitionId(2),
                    name: "Solid".into(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(3),
                        height: Dimension::new("5", 5.0).unwrap(),
                    },
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(1),
                    definition_id: DefinitionId(1),
                    name: "Shared A".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(2),
                    definition_id: DefinitionId(1),
                    name: "Shared B".into(),
                    transform: Transform::from_translation(30.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(3),
                    definition_id: DefinitionId(2),
                    name: "Independent".into(),
                    transform: Transform::from_translation(100.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        let before = document.current();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProfilePoints {
                    id: FeatureId(1),
                    points_mm: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 10.0], [0.0, 10.0]],
                },
            ]))
            .unwrap();
        let after_definition_edit = document.current();

        let scope = plan_incremental_exact_scope(&before, &after_definition_edit).unwrap();
        assert_eq!(
            scope.producers,
            BTreeSet::from([ProducerKey {
                definition_id: DefinitionId(1),
                feature_id: FeatureId(2),
            }])
        );
        assert_eq!(
            scope.collision_occurrences,
            BTreeSet::from([OccurrenceId(1), OccurrenceId(2)])
        );
        assert_eq!(scope.changed_feature_count, 1);
        assert_eq!(scope.changed_scene_occurrence_count, 0);
        let incremental = plan_incremental_exact_evaluation(
            &before,
            &after_definition_edit,
            Some(&exact_source(&before)),
        )
        .unwrap();
        assert!(incremental.baseline_reused);
        assert_eq!(
            incremental.selection,
            ExactEvaluationSelection::Scoped(scope.producers.clone())
        );
        let without_baseline =
            plan_incremental_exact_evaluation(&before, &after_definition_edit, None).unwrap();
        assert!(!without_baseline.baseline_reused);
        assert_eq!(without_baseline.selection, ExactEvaluationSelection::Full);
        assert_eq!(
            without_baseline.collision_occurrences,
            BTreeSet::from([OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)])
        );

        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id: OccurrenceId(3),
                    transform: Transform::from_translation(40.0, 0.0, 0.0).unwrap(),
                },
            ]))
            .unwrap();
        let transform_scope =
            plan_incremental_exact_scope(&after_definition_edit, &document.current()).unwrap();
        assert!(transform_scope.producers.is_empty());
        assert_eq!(
            transform_scope.collision_occurrences,
            BTreeSet::from([OccurrenceId(3)])
        );
        assert_eq!(transform_scope.changed_feature_count, 0);
        assert_eq!(transform_scope.changed_scene_occurrence_count, 1);
    }

    #[test]
    fn nested_shared_leaf_change_invalidates_every_component_root_once() {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Leaf".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(1),
                    definition_id: DefinitionId(1),
                    name: "Profile".into(),
                    kind: FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: DefinitionId(1),
                    name: "Solid".into(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(1),
                        height: Dimension::new("10", 10.0).unwrap(),
                    },
                },
                CanonicalCommand::CreateGroup {
                    id: GroupId(1),
                    name: "Component source".into(),
                    transform: Transform::identity(),
                    parent: None,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(1),
                    definition_id: DefinitionId(1),
                    name: "Nested leaf".into(),
                    transform: Transform::identity(),
                    parent: Some(GroupId(1)),
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        let converted = document
            .convert_group_to_component(GroupId(1), "Shared component")
            .unwrap();
        assert_eq!(converted.component_definition_id, DefinitionId(2));
        assert_eq!(converted.component_occurrence_id, OccurrenceId(2));
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(3),
                    definition_id: converted.component_definition_id,
                    name: "Shared component B".into(),
                    transform: Transform::from_translation(30.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tag: None,
                    visible: true,
                },
            ]))
            .unwrap();
        let before = document.current();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProfilePoints {
                    id: FeatureId(1),
                    points_mm: vec![[0.0, 0.0], [12.0, 0.0], [12.0, 10.0], [0.0, 10.0]],
                },
            ]))
            .unwrap();

        let scope = plan_incremental_exact_scope(&before, &document.current()).unwrap();
        assert_eq!(
            scope.producers,
            BTreeSet::from([ProducerKey {
                definition_id: DefinitionId(1),
                feature_id: FeatureId(2),
            }])
        );
        assert_eq!(
            scope.collision_occurrences,
            BTreeSet::from([OccurrenceId(2), OccurrenceId(3)])
        );
        assert_eq!(scope.changed_feature_count, 1);
        assert_eq!(scope.changed_scene_occurrence_count, 0);
    }
}
