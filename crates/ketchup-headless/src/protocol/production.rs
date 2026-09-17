use super::*;
use ketchup_core::document::{InstancePathStep, LocalGroupId, LocalOccurrenceId};
use ketchup_core::exact_validation::GeneralBodyParticipant;
use ketchup_core::fabrication::production::{
    HomagWoodwopAdapter, ProductionAdapter, instance_path_value,
};
use ketchup_core::fabrication::{WoodwopMprOptions, project_general_fabrication};
use ketchup_core::prismatic::TolerancePolicy;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathInput {
    root_occurrence_id: u64,
    #[serde(default)]
    steps: Vec<StepInput>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum StepInput {
    Group { id: u64 },
    Occurrence { id: u64 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Assignment {
    instance_path: PathInput,
    code: Option<String>,
}

impl Server {
    pub(super) fn production_codes(&self) -> Value {
        json!({"assignments": self.session.snapshot().production_codes().map(|(path, code)| {
            json!({"instance_path": instance_path_value(path), "code": code})
        }).collect::<Vec<_>>()})
    }

    pub(super) fn set_production_codes(&mut self, p: &Map<String, Value>) -> Result<Value> {
        if p.get("assignments")
            .and_then(Value::as_array)
            .is_some_and(|items| items.iter().any(|item| item.get("code").is_none()))
        {
            return Err(Error::invalid(
                "each assignment requires code; use explicit null to clear",
            ));
        }
        let assignments: Vec<Assignment> = serde_json::from_value(
            p.get("assignments")
                .cloned()
                .ok_or_else(|| Error::invalid("missing assignments"))?,
        )
        .map_err(|error| Error::invalid(error.to_string()))?;
        if assignments.is_empty() || assignments.len() > 512 {
            return Err(Error::invalid(
                "assignments must contain 1..512 physical part codes",
            ));
        }
        let mut paths = BTreeSet::new();
        let mut commands = Vec::new();
        for assignment in assignments {
            if assignment.instance_path.steps.len() > 64
                || assignment.instance_path.root_occurrence_id == 0
            {
                return Err(Error::invalid("invalid instance path"));
            }
            let mut path =
                InstancePath::root(OccurrenceId(assignment.instance_path.root_occurrence_id));
            for step in assignment.instance_path.steps {
                let (id, step) = match step {
                    StepInput::Group { id } => (id, InstancePathStep::Group(LocalGroupId(id))),
                    StepInput::Occurrence { id } => {
                        (id, InstancePathStep::Occurrence(LocalOccurrenceId(id)))
                    }
                };
                if id == 0 {
                    return Err(Error::invalid("instance step ID must be positive"));
                }
                path = path.with_step(step);
            }
            if !paths.insert(path.clone()) {
                return Err(Error::invalid("duplicate assignment path"));
            }
            commands.push(CanonicalCommand::SetProductionCode {
                instance_path: path,
                code: assignment.code,
            });
        }
        let proposal = self.session.plan_commands(CommandBatch::new(commands))?;
        self.session.apply_proposal(&proposal)?;
        self.initial_placeholder = false;
        Ok(self.state_result())
    }

    pub(super) fn production_job(&mut self, p: &Map<String, Value>) -> Result<Value> {
        let adapter_ids: Vec<String> =
            serde_json::from_value(p.get("adapters").cloned().ok_or_else(|| {
                Error::invalid("adapters must be an array; empty selects no machine")
            })?)
            .map_err(|error| Error::invalid(error.to_string()))?;
        if adapter_ids.len() > 1 || adapter_ids.iter().any(|id| id != "homag-woodwop4") {
            return Err(Error::invalid(
                "unknown or duplicate built-in production adapter",
            ));
        }
        let pocket_tool = match p.get("vertical_pocket_tool_number") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .filter(|value| (1..=999_999).contains(value))
                    .ok_or_else(|| Error::invalid("vertical pocket tool must be 1..999999"))?
                    as u32,
            ),
        };
        if pocket_tool.is_some() && adapter_ids.is_empty() {
            return Err(Error::invalid(
                "pocket tool option requires the HOMAG adapter",
            ));
        }
        let timeout_ms = uint(p, "timeout_ms")?;
        if !(1..=300_000).contains(&timeout_ms) {
            return Err(Error::invalid("timeout_ms must be 1..300000"));
        }
        if self.session.is_modified() || self.session.path().is_none() {
            return Err(Error::new(
                "unsaved_changes",
                "save the document and assigned codes before preparing a production job",
            ));
        }
        let budget = Duration::from_millis(timeout_ms);
        let started = Instant::now();
        self.session.evaluate_with_timeout(budget)?;
        let snapshot = self.session.snapshot();
        let tolerance = TolerancePolicy::default();
        let participants = snapshot
            .scene_query()
            .into_iter()
            .filter(|instance| instance.visible)
            .filter(|instance| {
                snapshot
                    .definition(instance.definition_id)
                    .is_some_and(|definition| {
                        definition.feature_ids().iter().any(|id| {
                            snapshot
                                .feature(*id)
                                .is_some_and(|feature| feature.kind().produces_body())
                        })
                    })
            })
            .map(|instance| {
                GeneralBodyParticipant::accept(
                    &snapshot,
                    self.session.exact_results(),
                    instance.instance_path,
                    tolerance,
                )
                .map_err(|error| {
                    Error::new(
                        "production_blocked",
                        format!("exact body unavailable: {error:?}"),
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let remaining = budget
            .checked_sub(started.elapsed())
            .filter(|time| !time.is_zero())
            .ok_or_else(|| Error::new("production_blocked", "production evaluation timed out"))?;
        let collision =
            ketchup_application::validation::fabrication_collision_validation_with_worker(
                &snapshot,
                &participants,
                self.session.container_data(),
                self.settings.exact_worker_path.clone(),
                remaining,
            )
            .map_err(|error| Error::new("production_blocked", error))?;
        let projection = project_general_fabrication(
            &snapshot,
            self.session.exact_results(),
            &collision.cases,
            &collision.report,
            tolerance,
        )
        .map_err(|error| Error::new("production_blocked", error.to_string()))?;
        let homag = HomagWoodwopAdapter {
            options: WoodwopMprOptions {
                vertical_pocket_tool_number: pocket_tool,
            },
        };
        let adapters: Vec<&dyn ProductionAdapter> = if adapter_ids.is_empty() {
            vec![]
        } else {
            vec![&homag]
        };
        let mut job = projection
            .production_job(&snapshot, &adapters)
            .map_err(|error| {
                Error::new(
                    "production_blocked",
                    format!("{error}; check assigned codes and adapter capabilities"),
                )
            })?;
        if started.elapsed() > budget {
            return Err(Error::new(
                "production_blocked",
                "production evaluation timed out",
            ));
        }
        job["source_mutation_epoch"] = json!(self.session.mutation_epoch());
        Ok(job)
    }
}
