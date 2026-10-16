//! Settings, production codes, evaluator nodes, overrides and feature parameter bindings.
use super::*;

pub(super) fn apply(
    edit: &mut BatchEdit,
    command: &CanonicalCommand,
) -> Result<(), CanonicalError> {
    match command {
        CanonicalCommand::SetFloorHeight { .. }
        | CanonicalCommand::SetContactJoints { .. }
        | CanonicalCommand::SetGroundedInstances { .. }
        | CanonicalCommand::SetOccurrenceGrounded { .. }
        | CanonicalCommand::SetTolerance { .. } => {
            support::apply_setting(&mut edit.product, command)?
        }
        CanonicalCommand::SetProductionCode {
            instance_path,
            code,
        } => {
            if let Some(code) = code {
                validate_production_code(code)?;
                let identity =
                    production_code_identity(&edit.product, instance_path).ok_or_else(|| {
                        CanonicalError::InvalidProductionCodePath(instance_path.clone())
                    })?;
                if edit
                    .product
                    .production_codes
                    .iter()
                    .any(|(path, assigned)| {
                        path != instance_path && assigned.eq_ignore_ascii_case(code)
                    })
                {
                    return Err(CanonicalError::DuplicateProductionCode(code.clone()));
                }
                edit.production_anchors
                    .entry(instance_path.clone())
                    .or_insert(Some(identity));
                edit.product
                    .production_codes
                    .insert(instance_path.clone(), code.clone());
            } else {
                edit.product.production_codes.remove(instance_path);
                edit.production_anchors.remove(instance_path);
                edit.rebound_production_paths.remove(instance_path);
            }
        }
        CanonicalCommand::CreateEvaluatorNode {
            id,
            name,
            dimension,
            dependencies,
        } => {
            create_evaluator_input(&mut edit.product, *id, name, dimension, dependencies)?;
        }
        CanonicalCommand::DeleteEvaluatorNode { id } => {
            edit.product
                .evaluator_nodes
                .remove(id)
                .ok_or(CanonicalError::NodeNotFound(*id))?;
        }
        CanonicalCommand::SetEvaluatorDimension { id, dimension } => {
            let existing = edit
                .product
                .evaluator_nodes
                .get(id)
                .ok_or(CanonicalError::NodeNotFound(*id))?;
            let replacement = EvaluatorNode::parameter(
                *id,
                existing.name.clone(),
                dimension.clone(),
                existing.dependencies.clone(),
            )
            .map_err(CanonicalError::Graph)?;
            edit.product
                .evaluator_nodes
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::RenameEvaluatorNode { id, name } => {
            let existing = edit
                .product
                .evaluator_nodes
                .get(id)
                .ok_or(CanonicalError::NodeNotFound(*id))?;
            let replacement = match &existing.kind {
                EvaluatorNodeKind::Parameter { value } => EvaluatorNode::parameter(
                    *id,
                    name.clone(),
                    value.clone(),
                    existing.dependencies.clone(),
                ),
                EvaluatorNodeKind::Expression { source, .. } => {
                    EvaluatorNode::expression(*id, name.clone(), source.clone())
                }
                EvaluatorNodeKind::Rule {
                    source, outputs, ..
                } => EvaluatorNode::rule(
                    *id,
                    name.clone(),
                    source.clone(),
                    existing.input_ports.clone(),
                    existing.output_ports.clone(),
                    outputs.clone(),
                    existing.allowed_parameters().to_vec(),
                ),
            }
            .map_err(CanonicalError::Graph)?;
            edit.product
                .evaluator_nodes
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::CreateExpressionNode {
            id,
            name,
            expression,
        } => {
            if edit.product.evaluator_nodes.contains_key(id) {
                return Err(CanonicalError::NodeAlreadyExists(*id));
            }
            let node = EvaluatorNode::expression(*id, name.clone(), expression.clone())
                .map_err(CanonicalError::Graph)?;
            edit.product.evaluator_nodes.insert(*id, Arc::new(node));
        }
        CanonicalCommand::CreateRuleNode {
            id,
            name,
            expression,
            input_ports,
            output_ports,
            outputs,
            override_parameters,
        } => {
            if edit.product.evaluator_nodes.contains_key(id) {
                return Err(CanonicalError::NodeAlreadyExists(*id));
            }
            let node = EvaluatorNode::rule(
                *id,
                name.clone(),
                expression.clone(),
                input_ports.clone(),
                output_ports.clone(),
                outputs.clone(),
                override_parameters.clone(),
            )
            .map_err(CanonicalError::Graph)?;
            edit.product.evaluator_nodes.insert(*id, Arc::new(node));
        }
        CanonicalCommand::SetNodeExpression { id, expression } => {
            let existing = edit
                .product
                .evaluator_nodes
                .get(id)
                .ok_or(CanonicalError::NodeNotFound(*id))?;
            let replacement = match &existing.kind {
                EvaluatorNodeKind::Expression { .. } => {
                    EvaluatorNode::expression(*id, existing.name.clone(), expression.clone())
                }
                EvaluatorNodeKind::Rule { outputs, .. } => EvaluatorNode::rule(
                    *id,
                    existing.name.clone(),
                    expression.clone(),
                    existing.input_ports.clone(),
                    existing.output_ports.clone(),
                    outputs.clone(),
                    existing.allowed_parameters().to_vec(),
                ),
                EvaluatorNodeKind::Parameter { .. } => {
                    return Err(CanonicalError::WrongNodeKind(*id));
                }
            }
            .map_err(CanonicalError::Graph)?;
            edit.product
                .evaluator_nodes
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::SetRuleOutputs { id, outputs } => {
            let existing = edit
                .product
                .evaluator_nodes
                .get(id)
                .ok_or(CanonicalError::NodeNotFound(*id))?;
            let EvaluatorNodeKind::Rule { source, .. } = &existing.kind else {
                return Err(CanonicalError::WrongNodeKind(*id));
            };
            let replacement = EvaluatorNode::rule(
                *id,
                existing.name.clone(),
                source.clone(),
                existing.input_ports.clone(),
                existing.output_ports.clone(),
                outputs.clone(),
                existing.allowed_parameters().to_vec(),
            )
            .map_err(CanonicalError::Graph)?;
            edit.product
                .evaluator_nodes
                .insert(*id, Arc::new(replacement));
        }
        CanonicalCommand::UpsertOverride(spec) => {
            let mut canonical = spec.clone();
            canonical.health =
                resolve_derived_identity(&edit.product.evaluator_nodes, &canonical.target);
            edit.product
                .overrides
                .insert(canonical.id, Arc::new(canonical));
        }
        CanonicalCommand::DeleteOverride { id } => {
            if edit.product.overrides.remove(id).is_none() {
                return Err(CanonicalError::OverrideNotFound(*id));
            }
        }
        CanonicalCommand::UpsertFeatureParameterBinding(binding) => {
            edit.product
                .feature_parameter_provenance
                .remove(&binding.target);
            edit.product
                .feature_parameter_bindings
                .insert(binding.target.clone(), Arc::new(binding.clone()));
        }
        CanonicalCommand::DeleteFeatureParameterBinding { target } => {
            if edit
                .product
                .feature_parameter_bindings
                .remove(target)
                .is_none()
            {
                return Err(CanonicalError::FeatureParameterBindingNotFound(
                    target.clone(),
                ));
            }
            edit.product.feature_parameter_provenance.remove(target);
        }
        CanonicalCommand::RecomputeFeatureParameters { identity, scope } => {
            let affected = match scope {
                FeatureParameterRecomputeScope::All => None,
                FeatureParameterRecomputeScope::AffectedBy(nodes) => {
                    Some(dependent_closure(&edit.product.evaluator_nodes, nodes))
                }
            };
            let report = recompute_feature_parameters(
                &mut edit.product,
                identity,
                affected.as_ref(),
                edit.previous_evaluation.as_ref(),
            )?;
            edit.previous_evaluation = Some(report.clone());
            edit.evaluation_identity = report.identity;
        }
        _ => {}
    }
    Ok(())
}
