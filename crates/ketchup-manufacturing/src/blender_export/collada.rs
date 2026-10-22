//! COLLADA 1.4.1 (`.dae`) export for SketchUp: the same current visible
//! bodies, tessellations and hierarchy as the GLB export, written in Ketchup's
//! own millimetres and Z-up. Equal bodies of one color share one component in
//! `library_nodes`, which SketchUp imports as instances of one definition.

use super::{
    GeometryKey, MeshGlbInstance, SceneLayout, add_with_limit, build_scene_nodes, geometry_key,
    validate_instances,
};
use crate::three_mf_export::BoundedString;
use ketchup_model::document::{Snapshot, Transform};
use ketchup_model::exact_product::{ExactProductError, MeshExportSource};
use ketchup_model::tolerance::limits;
use std::collections::BTreeMap;
use std::fmt::Write as _;

const MAX_DAE_EXPORT_BYTES: usize = 512 * 1024 * 1024;
/// The one material symbol of every triangle list; each component binds it.
const MATERIAL_SYMBOL: &str = "material";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaeExport {
    pub dae: String,
    pub loss_report: String,
}

struct Component<'a> {
    geometry: usize,
    material: Option<usize>,
    name: &'a str,
}

/// The COLLADA document of the current visible model. Refuses hidden or
/// stale occurrences like [`super::model_glb_export`].
pub fn model_dae_export(
    snapshot: &Snapshot,
    instances: &[MeshGlbInstance<'_>],
) -> Result<DaeExport, ExactProductError> {
    if instances
        .iter()
        .any(|instance| !instance.occurrence.visible)
    {
        return Err(ExactProductError::StaleResult);
    }
    validate_instances(snapshot, instances)?;

    let mut geometry_indices = BTreeMap::<GeometryKey, usize>::new();
    let mut geometries = Vec::<(MeshExportSource<'_>, &str)>::new();
    let mut material_indices = BTreeMap::<[u8; 3], usize>::new();
    let mut materials = Vec::<[u8; 3]>::new();
    let mut component_indices = BTreeMap::<(usize, Option<usize>), usize>::new();
    let mut components = Vec::<Component<'_>>::new();
    let mut instance_components = Vec::with_capacity(instances.len());
    let mut vertex_count = 0_usize;
    let mut triangle_count = 0_usize;
    for instance in instances {
        let source = instance.source;
        let name = instance.occurrence.definition_name.as_str();
        let key = geometry_key(source);
        let geometry = if let Some(index) = geometry_indices.get(&key) {
            *index
        } else {
            if source.vertex_count() == 0 || source.triangle_count() == 0 {
                return Err(ExactProductError::InvalidMeshExport);
            }
            vertex_count =
                add_with_limit(vertex_count, source.vertex_count(), limits::EXPORT_VERTICES)?;
            triangle_count = add_with_limit(
                triangle_count,
                source.triangle_count(),
                limits::EXPORT_TRIANGLES,
            )?;
            geometries.push((source, name));
            geometry_indices.insert(key, geometries.len() - 1);
            geometries.len() - 1
        };
        let material = instance.occurrence.color().map(|color| {
            *material_indices.entry(color).or_insert_with(|| {
                materials.push(color);
                materials.len() - 1
            })
        });
        let component = *component_indices
            .entry((geometry, material))
            .or_insert_with(|| {
                components.push(Component {
                    geometry,
                    material,
                    name,
                });
                components.len() - 1
            });
        instance_components.push(component);
    }
    let SceneLayout {
        nodes, root_nodes, ..
    } = build_scene_nodes(snapshot, instances, &instance_components)?;

    let mut xml = BoundedString::new(MAX_DAE_EXPORT_BYTES);
    write!(
        xml,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">\n  <asset>\n    <contributor>\n      <authoring_tool>Ketchup</authoring_tool>\n      <comments>source_digest={}</comments>\n    </contributor>\n    <created>1970-01-01T00:00:00Z</created>\n    <modified>1970-01-01T00:00:00Z</modified>\n    <unit meter=\"0.001\" name=\"millimeter\"/>\n    <up_axis>Z_UP</up_axis>\n  </asset>\n",
        snapshot.canonical_digest()
    )
    .map_err(over_limit)?;

    if !materials.is_empty() {
        xml.write_str("  <library_effects>\n").map_err(over_limit)?;
        for (index, color) in materials.iter().enumerate() {
            let [red, green, blue] = color.map(|channel| f64::from(channel) / 255.0);
            writeln!(
                xml,
                "    <effect id=\"effect-{index}\"><profile_COMMON><technique sid=\"common\"><lambert><diffuse><color>{red} {green} {blue} 1</color></diffuse></lambert></technique></profile_COMMON></effect>"
            )
            .map_err(over_limit)?;
        }
        xml.write_str("  </library_effects>\n  <library_materials>\n")
            .map_err(over_limit)?;
        for (index, [red, green, blue]) in materials.iter().enumerate() {
            writeln!(
                xml,
                "    <material id=\"material-{index}\" name=\"Ketchup_{red:02X}{green:02X}{blue:02X}\"><instance_effect url=\"#effect-{index}\"/></material>"
            )
            .map_err(over_limit)?;
        }
        xml.write_str("  </library_materials>\n")
            .map_err(over_limit)?;
    }

    let mut colored = vec![false; geometries.len()];
    for component in &components {
        colored[component.geometry] |= component.material.is_some();
    }
    xml.write_str("  <library_geometries>\n")
        .map_err(over_limit)?;
    for (index, (source, name)) in geometries.iter().enumerate() {
        write_geometry(&mut xml, index, *source, name, colored[index])?;
    }
    xml.write_str("  </library_geometries>\n  <library_nodes>\n")
        .map_err(over_limit)?;
    for (index, component) in components.iter().enumerate() {
        write!(xml, "    <node id=\"component-{index}\" name=\"").map_err(over_limit)?;
        write_name(&mut xml, component.name)?;
        write!(
            xml,
            "\"><instance_geometry url=\"#geometry-{}\">",
            component.geometry
        )
        .map_err(over_limit)?;
        if let Some(material) = component.material {
            write!(
                xml,
                "<bind_material><technique_common><instance_material symbol=\"{MATERIAL_SYMBOL}\" target=\"#material-{material}\"/></technique_common></bind_material>"
            )
            .map_err(over_limit)?;
        }
        xml.write_str("</instance_geometry></node>\n")
            .map_err(over_limit)?;
    }
    xml.write_str(
        "  </library_nodes>\n  <library_visual_scenes>\n    <visual_scene id=\"scene\" name=\"Ketchup_model\">\n",
    )
    .map_err(over_limit)?;

    enum Step {
        Open(usize),
        Close,
    }
    let mut steps = root_nodes
        .iter()
        .rev()
        .map(|index| Step::Open(*index))
        .collect::<Vec<_>>();
    while let Some(step) = steps.pop() {
        let Step::Open(index) = step else {
            xml.write_str("</node>\n").map_err(over_limit)?;
            continue;
        };
        let node = &nodes[index];
        write!(xml, "<node id=\"node-{index}\" name=\"").map_err(over_limit)?;
        write_name(&mut xml, &node.name)?;
        write!(
            xml,
            "\"><matrix sid=\"transform\">{}</matrix>",
            dae_matrix(node.transform)?
        )
        .map_err(over_limit)?;
        if let Some(component) = node.mesh {
            write!(xml, "<instance_node url=\"#component-{component}\"/>").map_err(over_limit)?;
        }
        xml.write_str("\n").map_err(over_limit)?;
        steps.push(Step::Close);
        steps.extend(node.children.iter().rev().map(|child| Step::Open(*child)));
    }
    xml.write_str(
        "    </visual_scene>\n  </library_visual_scenes>\n  <scene>\n    <instance_visual_scene url=\"#scene\"/>\n  </scene>\n</COLLADA>\n",
    )
    .map_err(over_limit)?;

    let canonical_mesh_count = instances
        .iter()
        .filter(|instance| instance.source.is_canonical())
        .count();
    let loss_report = format!(
        "authority=validated exact tessellations and canonical mesh bodies\nformat=COLLADA 1.4.1 (DAE)\nconversion=current-visible-model-to-instanced-mesh-scene\nunit=millimetres (no conversion)\naxis=Ketchup Z-up (no conversion)\nhierarchy=canonical global groups, component occurrences, local groups, and nested occurrences as nodes\nnames=COLLADA 1.4.1 NCName: characters an XML name does not allow (spaces, ², ½ among them) become _\ncomponents=equal bodies of one color share one geometry and one library node\nmaterials=resolved occurrence sRGB colors as diffuse lambert effects\nnormals=not written; faces follow the triangle winding\nedges=not written; SketchUp shows triangle edges unless its import merges coplanar faces\ntextures=not written\nsketchup_import_verified=false\neditability_loss=canonical features, parameters, rules, dimensions, constraints, joints, and Undo history are not preserved\ntopology_loss=exact topology, analytic surfaces, and durable face identity are not preserved\ntolerance_loss=exact geometry uses its accepted tessellation; canonical mesh vertices are preserved\nsource_digest={}\noccurrence_body_count={}\ncanonical_mesh_occurrence_count={canonical_mesh_count}\nunique_geometry_count={}\ncomponent_count={}\ntriangle_count={triangle_count}\nresource_vertex_limit={}\nresource_triangle_limit={}\n",
        snapshot.canonical_digest(),
        instances.len(),
        geometries.len(),
        components.len(),
        limits::EXPORT_VERTICES,
        limits::EXPORT_TRIANGLES,
    );
    Ok(DaeExport {
        dae: xml.into_inner(),
        loss_report,
    })
}

fn write_geometry(
    xml: &mut BoundedString,
    index: usize,
    source: MeshExportSource<'_>,
    name: &str,
    colored: bool,
) -> Result<(), ExactProductError> {
    let vertices = source.vertex_count();
    let triangles = source.triangle_count();
    write!(xml, "    <geometry id=\"geometry-{index}\" name=\"").map_err(over_limit)?;
    write_name(xml, name)?;
    write!(
        xml,
        "\"><mesh>\n      <source id=\"geometry-{index}-positions\"><float_array id=\"geometry-{index}-positions-array\" count=\"{}\">",
        vertices * 3
    )
    .map_err(over_limit)?;
    for vertex in 0..vertices {
        let point = source
            .vertex_position_mm(vertex)
            .ok_or(ExactProductError::InvalidMeshExport)?;
        if point.iter().any(|value| !value.is_finite()) {
            return Err(ExactProductError::InvalidMeshExport);
        }
        let separator = if vertex == 0 { "" } else { " " };
        write!(xml, "{separator}{} {} {}", point[0], point[1], point[2]).map_err(over_limit)?;
    }
    write!(
        xml,
        "</float_array><technique_common><accessor source=\"#geometry-{index}-positions-array\" count=\"{vertices}\" stride=\"3\"><param name=\"X\" type=\"float\"/><param name=\"Y\" type=\"float\"/><param name=\"Z\" type=\"float\"/></accessor></technique_common></source>\n      <vertices id=\"geometry-{index}-vertices\"><input semantic=\"POSITION\" source=\"#geometry-{index}-positions\"/></vertices>\n      <triangles count=\"{triangles}\""
    )
    .map_err(over_limit)?;
    if colored {
        write!(xml, " material=\"{MATERIAL_SYMBOL}\"").map_err(over_limit)?;
    }
    write!(
        xml,
        "><input semantic=\"VERTEX\" source=\"#geometry-{index}-vertices\" offset=\"0\"/><p>"
    )
    .map_err(over_limit)?;
    for triangle in 0..triangles {
        let [a, b, c] = source
            .triangle_indices(triangle)
            .ok_or(ExactProductError::InvalidMeshExport)?;
        if [a, b, c]
            .iter()
            .any(|vertex| !usize::try_from(*vertex).is_ok_and(|vertex| vertex < vertices))
        {
            return Err(ExactProductError::InvalidMeshExport);
        }
        let separator = if triangle == 0 { "" } else { " " };
        write!(xml, "{separator}{a} {b} {c}").map_err(over_limit)?;
    }
    xml.write_str("</p></triangles>\n    </mesh></geometry>\n")
        .map_err(over_limit)
}

/// The row-major matrix COLLADA `<matrix>` takes, in Ketchup's millimetres.
fn dae_matrix(transform: Transform) -> Result<String, ExactProductError> {
    if transform.affine().linear.is_singular() {
        return Err(ExactProductError::InvalidMeshExport);
    }
    Ok(transform
        .matrix()
        .iter()
        .map(f64::to_string)
        .collect::<Vec<_>>()
        .join(" "))
}

/// Whether `character` may start an XML 1.0 name without a colon (`NameStartChar`).
fn name_start_char(character: char) -> bool {
    matches!(character,
        'A'..='Z' | '_' | 'a'..='z' | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}'
        | '\u{F8}'..='\u{2FF}' | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}'
        | '\u{200C}'..='\u{200D}' | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}'
        | '\u{3001}'..='\u{D7FF}' | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}'
        | '\u{10000}'..='\u{EFFFF}')
}

/// Whether `character` may follow in an XML 1.0 name (`NameChar`): combining accents
/// and `·` among them, `²` and `½` not.
fn name_char(character: char) -> bool {
    name_start_char(character)
        || matches!(character,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

/// COLLADA 1.4.1 names are `xs:NCName`: the characters of an XML name without a colon,
/// not starting with a digit, `-`, `.` or a combining accent. Every other character
/// becomes `_`, so `Noha 1` is written `Noha_1` and `1. noha` `_1._noha`.
fn write_name(xml: &mut BoundedString, name: &str) -> Result<(), ExactProductError> {
    let mut written = String::with_capacity(name.len() + 1);
    for character in name.chars() {
        let allowed = if written.is_empty() {
            name_start_char(character)
        } else {
            name_char(character)
        };
        if !allowed && written.is_empty() && name_char(character) {
            written.push('_');
            written.push(character);
        } else {
            written.push(if allowed { character } else { '_' });
        }
    }
    if written.is_empty() {
        written.push('_');
    }
    xml.write_str(&written).map_err(over_limit)
}

fn over_limit(_: std::fmt::Error) -> ExactProductError {
    ExactProductError::ExportResourceLimit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(name: &str) -> String {
        let mut xml = BoundedString::new(1024);
        write_name(&mut xml, name).unwrap();
        xml.into_inner()
    }

    // Review 2026-10-21 P3: `²` and `½` are numeric to Rust but no XML name character, and a
    // combining accent of a decomposed letter is one.
    #[test]
    fn a_name_keeps_exactly_the_characters_an_xml_name_allows() {
        assert_eq!(written("Plocha m²"), "Plocha_m_");
        assert_eq!(written("½ dosky"), "__dosky");
        assert_eq!(written("Dosk\u{61}\u{301}"), "Dosk\u{61}\u{301}");
        assert_eq!(written("Noha 1"), "Noha_1");
        assert_eq!(written("1. noha"), "_1._noha");
        assert_eq!(written("Čelo·ľavé"), "Čelo·ľavé");
    }
}
