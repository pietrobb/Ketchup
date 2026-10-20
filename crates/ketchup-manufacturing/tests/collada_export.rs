use super::blender_glb_export::{current_package, glb_json, seeded_document};
use ketchup_geometry::linalg::{cross, dot};
use ketchup_manufacturing::blender_export::{
    ExactGlbInstance, MeshGlbInstance, exact_model_glb_export, model_dae_export,
};
use ketchup_model::exact_product::MeshExportSource;

fn exports(
    first: Option<[u8; 3]>,
    second: Option<[u8; 3]>,
) -> (ketchup_manufacturing::blender_export::DaeExport, Vec<u8>) {
    let document = seeded_document(first, second);
    let snapshot = document.current();
    let package = current_package(&snapshot);
    let occurrences = snapshot.scene_query();
    let instances = occurrences
        .iter()
        .map(|occurrence| MeshGlbInstance {
            source: MeshExportSource::Exact(&package),
            occurrence,
        })
        .collect::<Vec<_>>();
    let glb_instances = occurrences
        .iter()
        .map(|occurrence| ExactGlbInstance {
            package: &package,
            occurrence,
        })
        .collect::<Vec<_>>();
    (
        model_dae_export(&snapshot, &instances).unwrap(),
        exact_model_glb_export(&snapshot, &glb_instances)
            .unwrap()
            .glb,
    )
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text.find(start).unwrap() + start.len();
    &text[from..from + text[from..].find(end).unwrap()]
}

#[test]
fn equal_parts_share_one_geometry_as_two_instances_of_one_component_in_millimetres_z_up() {
    let (export, glb) = exports(Some([255, 0, 0]), Some([255, 0, 0]));
    let dae = &export.dae;
    assert!(dae.contains(
        "<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">"
    ));
    assert!(dae.contains("<unit meter=\"0.001\" name=\"millimeter\"/>"));
    assert!(dae.contains("<up_axis>Z_UP</up_axis>"));
    assert_eq!(dae.matches("<geometry ").count(), 1);
    assert_eq!(
        dae.matches("<instance_geometry url=\"#geometry-0\">")
            .count(),
        1
    );
    assert_eq!(
        dae.matches("<instance_node url=\"#component-0\"/>").count(),
        2
    );
    assert!(dae.contains("<node id=\"component-0\" name=\"Reusable_beam\">"));
    assert!(dae.contains(
        "name=\"Beam_A\"><matrix sid=\"transform\">1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1</matrix>"
    ));
    assert!(dae.contains(
        "name=\"Beam_B\"><matrix sid=\"transform\">1 0 0 20 0 1 0 30 0 0 1 40 0 0 0 1</matrix>"
    ));
    assert!(dae.contains("<diffuse><color>1 0 0 1</color></diffuse>"));
    assert!(dae.contains("target=\"#material-0\""));

    // The same triangles as the GLB of the same model.
    let json = glb_json(&glb);
    let mut index_accessors = json["meshes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|mesh| mesh["primitives"].as_array().unwrap())
        .filter(|primitive| primitive["mode"] == 4)
        .map(|primitive| primitive["indices"].as_u64().unwrap())
        .collect::<Vec<_>>();
    index_accessors.dedup();
    let glb_triangles = index_accessors
        .iter()
        .map(|accessor| {
            json["accessors"][*accessor as usize]["count"]
                .as_u64()
                .unwrap()
                / 3
        })
        .sum::<u64>();
    let dae_triangles = between(dae, "<triangles count=\"", "\"")
        .parse::<u64>()
        .unwrap();
    assert_eq!(dae_triangles, glb_triangles);
    assert!(export.loss_report.contains("format=COLLADA 1.4.1 (DAE)\n"));
    assert!(
        export
            .loss_report
            .contains(&format!("triangle_count={glb_triangles}\n"))
    );

    // A second color keeps the one geometry but makes a second component.
    let (export, _) = exports(Some([255, 0, 0]), Some([0, 0, 255]));
    assert_eq!(export.dae.matches("<geometry ").count(), 1);
    assert_eq!(
        export
            .dae
            .matches("<instance_node url=\"#component-0\"/>")
            .count(),
        1
    );
    assert_eq!(
        export
            .dae
            .matches("<instance_node url=\"#component-1\"/>")
            .count(),
        1
    );
    assert_eq!(export.dae.matches("<material id=").count(), 2);
}

#[test]
fn triangles_wind_outward_so_the_box_encloses_positive_volume() {
    let (export, _) = exports(None, None);
    let dae = &export.dae;
    assert!(!dae.contains("<library_materials>"));
    assert!(!dae.contains("material=\""));
    let positions = between(dae, "count=\"", "</float_array>");
    let positions = positions[positions.find('>').unwrap() + 1..]
        .split(' ')
        .map(|value| value.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    let indices = between(dae, "<p>", "</p>")
        .split(' ')
        .map(|value| value.parse::<usize>().unwrap())
        .collect::<Vec<_>>();
    let point = |index: usize| [0, 1, 2].map(|axis| positions[index * 3 + axis]);
    let volume = indices
        .chunks(3)
        .map(|triangle| {
            let [a, b, c] = [point(triangle[0]), point(triangle[1]), point(triangle[2])];
            dot(a, cross(b, c)) / 6.0
        })
        .sum::<f64>();
    // The 10 x 10 x 10 mm box, in millimetres, with every face turned outward.
    assert!((volume - 1000.0).abs() < 1.0e-6, "signed volume {volume}");
}
