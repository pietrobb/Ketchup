use super::*;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PersistentDimensionId(pub u64);

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PersistentDimensionTarget {
    FeatureParameter(FeatureParameterTarget),
    DerivedOutput(DerivedIdentity),
    ExactFeatureParameter {
        definition_id: DefinitionId,
        producer_feature_id: FeatureId,
        semantic_role: String,
        source_element_id: String,
        path: ParameterPath,
        value_type: ParameterValueType,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DimensionDisplayUnit {
    Millimetres,
    Centimetres,
    Inches,
}

impl DimensionDisplayUnit {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Millimetres => "mm",
            Self::Centimetres => "cm",
            Self::Inches => "in",
        }
    }

    #[must_use]
    pub fn from_millimetres(self, millimetres: f64) -> f64 {
        match self {
            Self::Millimetres => millimetres,
            Self::Centimetres => millimetres / 10.0,
            Self::Inches => millimetres / 25.4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DimensionPresentation {
    pub unit: DimensionDisplayUnit,
    pub decimal_places: u8,
}

impl DimensionPresentation {
    pub fn new(unit: DimensionDisplayUnit, decimal_places: u8) -> Result<Self, CanonicalError> {
        if decimal_places > 9 {
            return Err(CanonicalError::InvalidDimensionPresentation);
        }
        Ok(Self {
            unit,
            decimal_places,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PersistentDimension {
    pub id: PersistentDimensionId,
    pub name: String,
    pub target: PersistentDimensionTarget,
    pub presentation: DimensionPresentation,
}

impl PersistentDimension {
    pub fn new(
        id: PersistentDimensionId,
        name: impl Into<String>,
        target: PersistentDimensionTarget,
        presentation: DimensionPresentation,
    ) -> Result<Self, CanonicalError> {
        ensure_product_id(id.0)?;
        let name = name.into();
        ensure_name(&name)?;
        if matches!(
            &target,
            PersistentDimensionTarget::ExactFeatureParameter {
                definition_id,
                producer_feature_id,
                semantic_role,
                source_element_id,
                ..
            } if definition_id.0 == 0
                || producer_feature_id.0 == 0
                || semantic_role.is_empty()
                || source_element_id.is_empty()
        ) {
            return Err(CanonicalError::InvalidPersistentDimensionTarget);
        }
        Ok(Self {
            id,
            name,
            target,
            presentation,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DimensionReferenceHealth {
    Resolved,
    Ambiguous { segment_index: usize },
    Lost,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PersistentDimensionProjection {
    pub id: PersistentDimensionId,
    pub health: DimensionReferenceHealth,
    pub millimetres: Option<f64>,
    pub display_value: Option<f64>,
    pub display_text: Option<String>,
}

pub(super) fn validate_persistent_dimension(
    dimension: &PersistentDimension,
) -> Result<(), CanonicalError> {
    ensure_product_id(dimension.id.0)?;
    ensure_name(&dimension.name)?;
    DimensionPresentation::new(
        dimension.presentation.unit,
        dimension.presentation.decimal_places,
    )?;
    if matches!(
        &dimension.target,
        PersistentDimensionTarget::FeatureParameter(FeatureParameterTarget {
            value_type: ParameterValueType::Angle | ParameterValueType::Scalar,
            ..
        }) | PersistentDimensionTarget::ExactFeatureParameter {
            value_type: ParameterValueType::Angle | ParameterValueType::Scalar,
            ..
        }
    ) || matches!(
        &dimension.target,
        PersistentDimensionTarget::ExactFeatureParameter {
            definition_id,
            producer_feature_id,
            semantic_role,
            source_element_id,
            ..
        } if definition_id.0 == 0
            || producer_feature_id.0 == 0
            || semantic_role.is_empty()
            || source_element_id.is_empty()
    ) {
        return Err(CanonicalError::InvalidPersistentDimensionTarget);
    }
    Ok(())
}

pub(super) fn resolve_persistent_dimension(
    product: &ProductModel,
    dimension: &PersistentDimension,
) -> (DimensionReferenceHealth, Option<f64>) {
    match &dimension.target {
        PersistentDimensionTarget::FeatureParameter(target) => {
            let value = feature_parameter_value_bits(product, target).map(f64::from_bits);
            if value.is_some() {
                (DimensionReferenceHealth::Resolved, value)
            } else {
                (DimensionReferenceHealth::Lost, None)
            }
        }
        PersistentDimensionTarget::DerivedOutput(target) => {
            match resolve_derived_identity(&product.evaluator_nodes, target) {
                SlotResolution::Resolved => {
                    let value =
                        evaluate_graph(&product.evaluator_nodes, &EvaluationIdentity::default())
                            .ok()
                            .and_then(|report| {
                                report.outputs.get(target).map(|output| output.value)
                            });
                    if value.is_some() {
                        (DimensionReferenceHealth::Resolved, value)
                    } else {
                        (DimensionReferenceHealth::Lost, None)
                    }
                }
                SlotResolution::Ambiguous { segment_index } => {
                    (DimensionReferenceHealth::Ambiguous { segment_index }, None)
                }
                SlotResolution::Lost { .. } => (DimensionReferenceHealth::Lost, None),
            }
        }
        PersistentDimensionTarget::ExactFeatureParameter {
            definition_id,
            producer_feature_id,
            semantic_role,
            source_element_id,
            path,
            value_type,
        } => {
            let candidates = product
                .exact_reference_evidence
                .values()
                .filter(|reference| {
                    reference.document_id == product.document_id
                        && reference.definition_id == *definition_id
                        && reference.producer_feature_id == *producer_feature_id
                        && reference.semantic_role == *semantic_role
                        && reference.source_element_id == *source_element_id
                        && reference.has_valid_lineage()
                })
                .count();
            match candidates {
                0 => (DimensionReferenceHealth::Lost, None),
                1 => {
                    let value = feature_parameter_value_bits(
                        product,
                        &FeatureParameterTarget {
                            feature_id: *producer_feature_id,
                            path: path.clone(),
                            value_type: *value_type,
                        },
                    )
                    .map(f64::from_bits);
                    if value.is_some() {
                        (DimensionReferenceHealth::Resolved, value)
                    } else {
                        (DimensionReferenceHealth::Lost, None)
                    }
                }
                _ => (
                    DimensionReferenceHealth::Ambiguous { segment_index: 0 },
                    None,
                ),
            }
        }
    }
}
