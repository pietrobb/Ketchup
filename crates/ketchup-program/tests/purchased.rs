//! `imported()` and the library's `purchased()`: bought parts whose body is a
//! STEP file, placed by the program.
use ketchup_program::bom::bom;
use ketchup_program::model::ProgramPartBody;
use ketchup_program::run;
use std::collections::BTreeMap;

const HINGES: &str = "door_h = 2000\n\
    for (i, z) in enumerate([100, door_h - 100]):\n\
    \x20   purchased(\"hinge %d\" % i, \"hettich/9237903.stp\", \"9237903\", \"Hettich\",\n\
    \x20             at = (18, 0, z), z = (-1, 0, 0), x = (0, 0, 1))\n\
    board(\"door\", (600, 18, door_h))\n";

fn model(source: &str) -> ketchup_program::model::ProgramModel {
    run("wardrobe.star", source, &BTreeMap::new())
        .unwrap()
        .0
        .model
}

fn error(source: &str) -> String {
    run("wardrobe.star", source, &BTreeMap::new())
        .unwrap_err()
        .message
}

#[test]
fn purchased_places_the_file_where_the_parameters_put_it() {
    let model = model(HINGES);
    let hinge = model.part("hinge 1").unwrap();
    let ProgramPartBody::Imported(import) = &hinge.body else {
        panic!("not read from a file: {:?}", hinge.body);
    };
    assert_eq!(import.file, "hettich/9237903.stp");
    assert_eq!(import.part, None);
    assert_eq!(hinge.at_mm, [18.0, 0.0, 1900.0]);
    assert_eq!(hinge.attributes["designation"], "Hettich 9237903");
    // Local z along world -x.
    assert!(
        (hinge.rotation[0][2] + 1.0).abs() < 1e-9,
        "{:?}",
        hinge.rotation
    );
    let moved = model_with_height(2400);
    assert_eq!(moved.part("hinge 1").unwrap().at_mm, [18.0, 0.0, 2300.0]);
}

fn model_with_height(height: i32) -> ketchup_program::model::ProgramModel {
    model(&HINGES.replace("door_h = 2000", &format!("door_h = {height}")))
}

#[test]
fn bought_parts_are_counted_in_the_hardware_list_and_never_cut() {
    let model = model(&format!(
        "{HINGES}purchased(\"runners\", \"hettich/runner.stp\", \"9237906\", \"Hettich\", qty = 2)\n"
    ));
    let bom = bom(&model);
    let row = |item: &str| {
        bom.hardware
            .iter()
            .find(|row| row.item == item)
            .unwrap_or_else(|| panic!("{item} missing: {:?}", bom.hardware))
    };
    assert_eq!(row("Hettich 9237903").count, 2);
    assert_eq!(row("Hettich 9237903").attributes["supplier"], "Hettich");
    assert_eq!(row("Hettich 9237906").count, 2);
    let cut: Vec<_> = bom.cut_list.iter().flat_map(|row| &row.parts).collect();
    assert_eq!(cut, ["door"], "{:?}", bom.cut_list);
    assert!(
        model
            .parts
            .iter()
            .filter(|part| part.imported().is_some())
            .all(|part| !part.made())
    );
}

#[test]
fn n_bought_parts_of_one_article_are_n_of_that_article_in_the_hardware_list() {
    let model = model(
        "for i in range(5):\n\
         \x20   purchased(\"hinge %d\" % i, \"hettich/9237903.stp\", \"9237903\", \"Hettich\",\n\
         \x20             at = (18, 0, 100 + 400 * i))\n",
    );
    let bom = bom(&model);
    assert_eq!(bom.hardware.len(), 1, "{:?}", bom.hardware);
    let row = &bom.hardware[0];
    assert_eq!(row.item, "Hettich 9237903");
    assert_eq!(row.count, 5);
    assert_eq!(row.parts.len(), 5, "{:?}", row.parts);
    assert!(bom.cut_list.is_empty(), "{:?}", bom.cut_list);
}

#[test]
fn a_file_outside_the_program_folder_is_refused() {
    for file in [
        "../secret.step",
        "C:/parts/hinge.step",
        "/etc/hinge.step",
        "a/../../b.step",
        "..\\secret.step",
        "\\\\server\\share\\hinge.step",
        "C:hinge.step",
    ] {
        let message = error(&format!("imported(\"h\", {file:?})\n"));
        assert!(
            message.contains("outside the folder of the program's file"),
            "{file}: {message}"
        );
    }
    // Down and back up inside the folder is fine.
    model("imported(\"h\", \"a/../b.step\")\n");
}

#[test]
fn only_step_files_are_read() {
    let message = error("imported(\"h\", \"hinge.dxf\")\n");
    assert!(message.contains("is not a STEP file"), "{message}");
    model("imported(\"h\", \"HINGE.STP\")\n");
}

#[test]
fn a_bought_part_takes_no_operation_and_is_no_tool() {
    let message = error(
        "h = imported(\"h\", \"h.step\")\nhole(h, \"start\", at = (0, 0), diameter = 5, depth = 3)\n",
    );
    assert!(message.contains("read from a STEP file"), "{message}");
    let message = error(
        "h = imported(\"h\", \"h.step\")\nb = board(\"b\", (100, 18, 100))\nsubtract(b, h)\n",
    );
    assert!(message.contains("cannot be a tool"), "{message}");
}

#[test]
fn purchased_checks_what_the_part_is_ordered_by() {
    let message = error("purchased(\"h\", \"h.step\", \"\", \"Hettich\")\n");
    assert!(message.contains("article must be"), "{message}");
    let message = error("purchased(\"h\", \"h.step\", \"1\", \"Hettich\", qty = 0)\n");
    assert!(message.contains("qty must be"), "{message}");
}
