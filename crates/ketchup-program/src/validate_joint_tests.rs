use super::*;

/// A joint whose boxes touch is settled by the boxes, and support groups
/// jointed parts before measuring contacts, so no face of either is measured
/// (a 142-part model with 66 joints spent 10 s on them in a debug build).
#[test]
fn touching_jointed_parts_are_checked_without_measuring_faces() {
    let source = "\
a = box(\"a\", (600, 400, 18))
b = box(\"b\", (600, 18, 400), at = (0, 0, 18))
joint(a, b, kind = \"glue\")
";
    let model = crate::evaluate("joint.star", source, &std::collections::BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    let mut faces = ContactFaces::default();
    let issues = validate_with_faces(&model, &ExactShapes::default(), &mut faces);
    let kinds: Vec<_> = issues.iter().map(|issue| issue.kind).collect();
    assert!(!kinds.contains(&"joint_without_contact"), "{kinds:?}");
    assert!(!kinds.contains(&"floating_part"), "{kinds:?}");
    assert_eq!(faces.parts_measured(), 0);
}
