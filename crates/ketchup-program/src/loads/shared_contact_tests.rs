use super::*;
use crate::exact::ExactShapes;

/// The loads take the contacts the load path measured instead of
/// measuring every pair a second time, and come out the same.
#[test]
fn the_loads_reuse_the_contacts_the_load_path_measured() {
    let source = "
load_path(only = [\"frame\"])
self_weight([\"frame\"])
box(\"post a\", (100, 100, 2000), material = \"C24\", tags = [\"frame\"])
box(\"post b\", (100, 100, 2000), at = (2900, 0, 0), material = \"C24\", tags = [\"frame\"])
box(\"beam\", (3000, 100, 200), at = (0, 0, 2000), material = \"C24\", tags = [\"frame\"])
";
    let model = crate::evaluate("shared.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    let mut faces = ContactFaces::default();
    crate::validate::validate_with_faces(&model, &ExactShapes::default(), &mut faces);
    let (measured, reused) = faces.measured_and_reused();
    assert!(measured > 0);
    let shared = loads_with(&model, &mut faces);
    // Every pair the loads look at was already measured.
    assert_eq!(faces.measured_and_reused(), (measured, reused + measured));
    assert_eq!(shared.members.len(), 3);
    assert_eq!(shared, loads(&model));
}
