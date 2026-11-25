//! Review 2026-11-25 R1125og-DXF-1001: an underlay whose lines only version 7 of the blob
//! holds (a text's font, a polyline's width) is not drawn by Kečup 0.3.0 to 0.3.3, which
//! read versions 1 to 6. A file with one is container schema 3, which every release
//! refuses as newer, instead of opening there with its underlay left blank; a file of
//! lines alone stays schema 1.
use ketchup_model::document::DocumentStore;
use ketchup_model::graph::sha256_bytes;
use ketchup_model::import::{DxfImportOptions, read_dxf_underlay};
use ketchup_model::persistence::{self, LoadDisposition};
use ketchup_model::underlay::plan_underlay;

/// A drawing as ezdxf writes one: the Standard text style on the font file `txt`.
fn drawing(entities: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nSTYLE\n70\n1\n\
         0\nSTYLE\n2\nStandard\n70\n0\n40\n0\n41\n1\n50\n0\n71\n0\n42\n2.5\n3\ntxt\n4\n\n\
         0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

const WALL: &str = "0\nLINE\n8\nwalls\n10\n0\n20\n0\n11\n6000\n21\n0\n";
const ROOM_NAME: &str = "0\nTEXT\n8\nnotes\n7\nStandard\n10\n500\n20\n500\n40\n250\n1\nKitchen\n";

/// The lines of `entities` saved as an underlay of a document: the blob and the file.
fn saved(entities: &str) -> (Vec<u8>, Vec<u8>) {
    let underlay = read_dxf_underlay(&drawing(entities), DxfImportOptions::new(None)).unwrap();
    let blob = underlay.lines.encode();
    let mut document = DocumentStore::new();
    let (batch, _) = plan_underlay(
        &document.current(),
        "Plan",
        &underlay.layers,
        &underlay.hidden_layers,
        sha256_bytes(&blob),
        underlay.offset_mm,
        None,
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let mut container = persistence::ContainerData::default();
    container.insert_blob(blob.clone()).unwrap();
    let file = persistence::save_container(&document.current(), &container).unwrap();
    (blob, file)
}

fn schema(file: &[u8]) -> u16 {
    u16::from_le_bytes([file[10], file[11]])
}

#[test]
fn a_text_in_a_styles_font_makes_the_file_newer_than_the_releases_read() {
    let (blob, file) = saved(&format!("{WALL}{ROOM_NAME}"));
    assert!(blob.starts_with(b"KUNDRLY7"), "the font needs version 7");
    assert_eq!(schema(&file), 3, "every release refuses it as newer");

    let opened = persistence::load(&file).unwrap();
    assert_eq!(opened.disposition(), LoadDisposition::EditableLossless);
    let again = persistence::save_container(&opened.snapshot(), opened.container_data()).unwrap();
    assert_eq!(again, file, "Open and Save keep it newer");
}

#[test]
fn lines_alone_stay_in_a_file_every_release_reads() {
    let (blob, file) = saved(WALL);
    assert!(!blob.starts_with(b"KUNDRLY7"));
    assert_eq!(schema(&file), 1);
}
