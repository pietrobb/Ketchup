use std::io::Cursor;

use acadrust::objects::ObjectType;
use acadrust::{DxfReader, EntityType};

use super::*;

/// An A3 sheet with a frame and a title on paper and the 8 x 6 m outline of a plan in model
/// space where it lies, shown at 1:100.
fn plan_sheet() -> Sheet {
    let layer = |name: &str, color, lineweight, pattern| SheetLayer {
        name: name.to_owned(),
        color,
        lineweight,
        pattern,
    };
    Sheet {
        layout: "01 Pôdorys prízemia".to_owned(),
        paper_mm: [420.0, 297.0],
        layers: vec![
            layer("K-FRAME", 7, 70, LinePattern::Continuous),
            layer("K-TEXT", 7, 25, LinePattern::Continuous),
            layer("K-CUT", 7, 50, LinePattern::Continuous),
            layer("K-CUT-FILL", 253, 0, LinePattern::Continuous),
            layer("K-ABOVE", 7, 18, LinePattern::Dashed),
        ],
        model: vec![
            SheetEntity::Fill {
                layer: "K-CUT-FILL".to_owned(),
                loops: vec![
                    vec![[0.0, 0.0], [8000.0, 0.0], [8000.0, 6000.0], [0.0, 6000.0]],
                    vec![
                        [300.0, 300.0],
                        [7700.0, 300.0],
                        [7700.0, 5700.0],
                        [300.0, 5700.0],
                    ],
                ],
                color: Some([110, 85, 60]),
            },
            SheetEntity::Polyline {
                layer: "K-CUT".to_owned(),
                points: vec![[0.0, 0.0], [8000.0, 0.0], [8000.0, 6000.0], [0.0, 6000.0]],
                closed: true,
                pen: Pen::default(),
            },
            SheetEntity::Polyline {
                layer: "K-ABOVE".to_owned(),
                points: vec![[-500.0, -500.0], [8500.0, -500.0]],
                closed: false,
                pen: Pen::default(),
            },
            SheetEntity::Text {
                layer: "K-TEXT".to_owned(),
                at: [4000.0, 3000.0],
                value: "Obývačka 24,30 m²".to_owned(),
                height: 250.0,
                angle_deg: 0.0,
                align: TextAlign::Middle,
            },
        ],
        paper: vec![
            SheetEntity::Polyline {
                layer: "K-FRAME".to_owned(),
                points: vec![[20.0, 10.0], [410.0, 10.0], [410.0, 287.0], [20.0, 287.0]],
                closed: true,
                pen: Pen::default(),
            },
            SheetEntity::Text {
                layer: "K-TEXT".to_owned(),
                at: [30.0, 20.0],
                value: "Pôdorys prízemia 1:100, sklon 5 %".to_owned(),
                height: 5.0,
                angle_deg: 0.0,
                align: TextAlign::Start,
            },
        ],
        viewports: vec![SheetViewport {
            paper_min: [30.0, 40.0],
            paper_max: [130.0, 120.0],
            view_center: [4000.0, 3000.0],
            twist_deg: 0.0,
            scale: 100.0,
        }],
    }
}

fn read(dxf: &[u8]) -> CadDocument {
    DxfReader::from_reader(Cursor::new(dxf.to_vec()))
        .and_then(DxfReader::read)
        .expect("the sheet's DXF reads")
}

#[test]
fn a_sheet_is_a_dxf_2013_with_its_views_in_model_space_and_a_layout_of_its_paper() {
    let sheet = plan_sheet();
    let dxf = sheet_dxf(&sheet).expect("the sheet is written");
    let text = String::from_utf8(dxf.clone()).expect("a DXF 2013 is UTF-8");
    assert!(
        text.contains("$ACADVER\r\n  1\r\nAC1027\r\n"),
        "AutoCAD 2013"
    );
    assert!(text.contains("Obývačka 24,30 m²"), "letters stay letters");
    assert!(text.contains("sklon 5 %%%\r\n"), "a percent sign stays one");

    let document = read(&dxf);
    let layout = document
        .objects
        .values()
        .find_map(|object| match object {
            ObjectType::Layout(layout) if layout.name == sheet.layout => Some(layout.clone()),
            _ => None,
        })
        .expect("the layout is named after the sheet");
    assert_eq!((layout.paper_width, layout.paper_height), (420.0, 297.0));

    let model = document.model_space_entities().collect::<Vec<_>>();
    assert_eq!(
        model.len(),
        4,
        "the fill, the outline, the line above and the text"
    );
    let outline = model
        .iter()
        .find_map(|entity| match entity {
            EntityType::LwPolyline(polyline) if polyline.common.layer == "K-CUT" => {
                Some(polyline.clone())
            }
            _ => None,
        })
        .expect("the outline is in model space");
    let xs = outline
        .vertices
        .iter()
        .map(|v| v.location.x)
        .collect::<Vec<_>>();
    assert_eq!(xs, [0.0, 8000.0, 8000.0, 0.0], "at 1:1 where it lies");

    let viewports = document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Viewport(viewport) if viewport.id > 1 => Some(viewport.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(viewports.len(), 1);
    assert!((viewports[0].scale() - 0.01).abs() < 1e-9, "1:100");
    assert_eq!(
        (viewports[0].view_center.x, viewports[0].view_center.y),
        (4000.0, 3000.0)
    );

    let dashed = document.layers.get("K-ABOVE").expect("the layer is there");
    assert_eq!(dashed.line_type, "DASHED");
    assert_eq!(dashed.line_weight, LineWeight::Value(18));
    assert!(document.line_types.get("DASHED").is_some());
}

#[test]
fn a_sheet_is_a_dwg_of_the_same_drawing() {
    let dwg = sheet_dwg(&plan_sheet()).expect("the sheet is written as DWG");
    let dxf = crate::dwg_to_whole_dxf(&dwg, 64 << 20).expect("the DWG reads back");
    let document = read(&dxf.dxf);
    assert_eq!(document.model_space_entities().count(), 4);
}

#[test]
fn a_pen_width_becomes_the_nearest_lineweight_autocad_has() {
    assert_eq!(lineweight(0.18), 18);
    assert_eq!(lineweight(0.5), 50);
    assert_eq!(lineweight(0.05), 5);
    assert_eq!(lineweight(0.46), 50, "between 40 and 50, nearer 50");
    assert_eq!(lineweight(3.0), 211);
}
