//! The box an import records around all of an imported body. The collision
//! check passes over a pair whose boxes are apart without reading either
//! source, so the box must hold the whole body: OCCT builds it from the
//! control points of the body's surfaces and curves and its tolerances, not
//! from samples, and a body read again must still lie inside the box its
//! import recorded.

use super::{ExactBRepOperation, ExactOpOutput, bounds_mm, imported_result_matches};

/// The box OCCT finds around all of an imported body, when it holds the
/// body's tightest bounds.
pub(super) fn enclosing_bounds_mm(output: &ExactOpOutput) -> Option<[[f64; 3]; 2]> {
    output
        .body
        .enclosing_bounds_mm()
        .filter(|enclosing| ketchup_model::import::encloses(bounds_mm(output), Some(*enclosing)))
}

/// Whether a body read for an `ImportedExact` node is the one its node names:
/// the same result, inside the box the node carries if it carries one.
pub(super) fn matches(
    operation: &ExactBRepOperation,
    source_sha256: &str,
    output: &ExactOpOutput,
    result_fingerprint: &str,
) -> bool {
    holds(operation, output) && imported_result_matches(source_sha256, output, result_fingerprint)
}

/// Whether a body read for an `ImportedExact` node lies inside the box the
/// node carries, or the node carries none.
fn holds(operation: &ExactBRepOperation, output: &ExactOpOutput) -> bool {
    let ExactBRepOperation::ImportedExact {
        enclosing_bounds_bits,
        ..
    } = operation
    else {
        return true;
    };
    ketchup_model::import::encloses(
        bounds_mm(output),
        enclosing_bounds_bits.map(|bounds| bounds.map(|corner| corner.map(f64::from_bits))),
    )
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::holds;

    #[test]
    fn every_part_of_an_assembly_lies_inside_the_box_its_import_found() {
        let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpora/r0/step/independent-xde-assembly.step");
        let bytes = std::fs::read(&corpus).unwrap();
        let step = SourceFile {
            sha256: ketchup_model::graph::sha256_hex(&bytes),
            path: corpus.clone(),
        };
        let backend = ExactBackend::new();
        let WorkerReply::Xde(evidence) =
            m21_step_xde_inspection_response(&backend, &mut SourceDocument::default(), &step)
                .unwrap()
        else {
            panic!("the STEP source inspects as XDE");
        };
        let document = backend.open_step_xde(&corpus.to_string_lossy()).unwrap();
        assert!(!evidence.parts.is_empty());
        for part in &evidence.parts {
            let enclosing = part.enclosing_bounds_mm.unwrap();
            // It holds the tightest bounds and is not much larger.
            assert!(
                ketchup_model::import::encloses(part.bounds_mm, Some(enclosing)),
                "{}: {enclosing:?} {:?}",
                part.name,
                part.bounds_mm
            );
            for (outer, inner) in enclosing
                .iter()
                .flatten()
                .zip(part.bounds_mm.iter().flatten())
            {
                assert!(
                    (outer - inner).abs() < 0.01,
                    "{}: {outer} {inner}",
                    part.name
                );
            }
            // A body read again is checked against the box its node carries.
            let output = document.part(part.index).unwrap();
            let node = |bounds: [[f64; 3]; 2]| ExactBRepOperation::ImportedExact {
                source_sha256: [1; 32],
                source_byte_len: 1,
                result_fingerprint: part.result_fingerprint.clone(),
                source_format: ExactBRepImportedFormat::Step,
                source_part_index: Some(part.index),
                enclosing_bounds_bits: Some(bounds.map(|corner| corner.map(f64::to_bits))),
            };
            assert!(holds(&node(enclosing), &output), "{}", part.name);
            let mut short = enclosing;
            short[1][0] = part.bounds_mm[1][0] - 1.0;
            assert!(!holds(&node(short), &output), "{}", part.name);
        }
    }
}
