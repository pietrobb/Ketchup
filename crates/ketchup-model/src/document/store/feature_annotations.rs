//! What the product keeps about a feature besides the feature: its parameter
//! bindings and the provenance of the values applied through them, and the
//! threads tapped into its hole or cut on its body. A feature that goes takes
//! them along; a feature copied in the same frame carries them over (M-11).
use super::*;

/// Every [`ProductModel`] field keyed by a feature. The test below fails when
/// a field keyed by a feature is added without being handled here.
#[cfg(test)]
const FEATURE_KEYED: &[&str] = &[
    "feature_parameter_bindings",
    "feature_parameter_provenance",
    "hole_threads",
    "shaft_threads",
];

/// Drops what the product keeps about the deleted feature `id`.
pub(super) fn forget_feature_annotations(product: &mut ProductModel, id: FeatureId) {
    product
        .feature_parameter_bindings
        .retain(|target, _| target.feature_id != id);
    product
        .feature_parameter_provenance
        .retain(|target, _| target.feature_id != id);
    product.hole_threads.remove(&id);
    product.shaft_threads.remove(&id);
}

/// Records on each copy what the product keeps about its source feature;
/// `mapping` takes a source feature to its copy in the same frame.
pub(in crate::document) fn copy_feature_annotations(
    product: &mut ProductModel,
    mapping: &BTreeMap<FeatureId, FeatureId>,
) {
    let copied = |target: &FeatureParameterTarget| {
        mapping
            .get(&target.feature_id)
            .map(|copy| FeatureParameterTarget {
                feature_id: *copy,
                ..target.clone()
            })
    };
    let bindings = product
        .feature_parameter_bindings
        .values()
        .filter_map(|binding| {
            copied(&binding.target).map(|target| {
                (
                    target.clone(),
                    Arc::new(FeatureParameterBinding {
                        target,
                        derived_from: binding.derived_from.clone(),
                    }),
                )
            })
        })
        .collect::<Vec<_>>();
    let provenance = product
        .feature_parameter_provenance
        .iter()
        .filter_map(|(target, provenance)| {
            copied(target).map(|target| (target, provenance.clone()))
        })
        .collect::<Vec<_>>();
    let hole_threads = mapping
        .iter()
        .filter_map(|(source, copy)| Some((*copy, product.hole_threads.get(source)?.clone())))
        .collect::<Vec<_>>();
    let shaft_threads = mapping
        .iter()
        .filter_map(|(source, copy)| Some((*copy, product.shaft_threads.get(source)?.clone())))
        .collect::<Vec<_>>();
    product.feature_parameter_bindings.extend(bindings);
    product.feature_parameter_provenance.extend(provenance);
    product.hole_threads.extend(hole_threads);
    product.shaft_threads.extend(shaft_threads);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name and type of every field of `ProductModel`.
    fn product_fields() -> Vec<(String, String)> {
        let source = include_str!("../../document.rs");
        let body = source
            .split_once("pub(crate) struct ProductModel {")
            .and_then(|(_, rest)| rest.split_once("\n}"))
            .map(|(body, _)| body)
            .expect("ProductModel is declared in document.rs");
        let mut fields = Vec::new();
        let mut current: Option<(String, String)> = None;
        for line in body.lines().map(str::trim) {
            if let Some(field) = line.strip_prefix("pub(crate) ") {
                fields.extend(current.take());
                let (name, ty) = field.split_once(':').expect("a field has a type");
                current = Some((name.trim().to_owned(), ty.trim().to_owned()));
            } else if let Some((_, ty)) = &mut current
                && !line.starts_with("//")
                && !line.starts_with('#')
            {
                ty.push_str(line);
            }
        }
        fields.extend(current);
        fields
    }

    #[test]
    fn every_product_field_keyed_by_a_feature_is_forgotten_and_copied_with_it() {
        let fields = product_fields();
        assert!(fields.len() > 40, "{fields:?}");
        let keyed = fields
            .iter()
            // The features themselves are copied and deleted by their callers.
            .filter(|(name, ty)| {
                name != "features"
                    && (ty.starts_with("BTreeMap<FeatureId,")
                        || ty.starts_with("BTreeMap<FeatureParameterTarget,"))
            })
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            keyed, FEATURE_KEYED,
            "handle a product field keyed by a feature in feature_annotations.rs"
        );
    }
}
