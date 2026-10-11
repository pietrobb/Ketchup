//! Author dimensions in the 3D view: anchors placed in the world, the source
//! value in its display unit, and which ones the active scene shows.

use crate::model::Model;
use ketchup_geometry::linalg::{Affine3, Vec3};
use ketchup_view_format::{Anchor, LengthUnit, LocalDimensions, Section};

/// One dimension ready to draw, in source Z-up millimetres.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub id: u64,
    pub from: [f64; 3],
    pub to: [f64; 3],
    /// Displacement of the dimension line from the measured points.
    pub offset: [f64; 3],
    /// The authoritative source value, never a measurement of the mesh.
    pub text: String,
}

/// Dimensions the active scene switches on (all of them without a scene),
/// leaving out those anchored to a hidden component or cut away by `section`.
pub fn placed(
    model: &Model,
    scene: Option<usize>,
    visible: &[bool],
    section: Option<&Section>,
) -> Vec<Placed> {
    let manifest = model.manifest();
    let shown = scene.map(|i| &manifest.scenes[i].visible_dimensions);
    manifest
        .dimensions
        .iter()
        .filter(|dimension| shown.is_none_or(|ids| ids.contains(&dimension.id)))
        .filter_map(|dimension| {
            let from = world_point(model, &dimension.from, visible)?;
            let to = world_point(model, &dimension.to, visible)?;
            let removed = |point: [f64; 3]| {
                section.is_some_and(|s| Vec3::from(s.normal).dot(point.into()) > s.offset_mm)
            };
            if removed(from) || removed(to) {
                return None;
            }
            Some(Placed {
                id: dimension.id,
                from,
                to,
                offset: dimension.label_offset_mm,
                text: length_text(dimension.value_mm, &dimension.display_unit),
            })
        })
        .collect()
}

/// The anchor in world space, or `None` when its component is hidden.
pub(crate) fn world_point(model: &Model, anchor: &Anchor, visible: &[bool]) -> Option<[f64; 3]> {
    let Some(path) = &anchor.occurrence else {
        return Some(anchor.point_mm);
    };
    let index = model
        .manifest()
        .occurrences
        .iter()
        .position(|occurrence| &occurrence.path == path)?;
    if !visible.get(index).copied().unwrap_or(false) {
        return None;
    }
    let placement = Affine3::from_column_major(model.occurrence_matrices_mm[index]);
    Some(placement.transform_point(anchor.point_mm.into()).into())
}

/// A world point held on occurrence `index`, in that component's own axes,
/// so it stays on the same spot of the part.
pub(crate) fn anchor_on(model: &Model, index: usize, world_mm: [f64; 3]) -> Option<Anchor> {
    let placement = Affine3::from_column_major(*model.occurrence_matrices_mm.get(index)?);
    let local = placement.invert().ok()?;
    Some(Anchor {
        occurrence: Some(model.manifest().occurrences[index].path.clone()),
        point_mm: local.transform_point(world_mm.into()).into(),
    })
}

/// The picked component's overall size along its own axes: three dimensions
/// on its local box, labelled with the source size. Components whose package
/// carries no size get none rather than a size measured from the mesh.
pub fn overall(model: &Model, index: usize) -> Vec<Placed> {
    let manifest = model.manifest();
    let Some(sizes) = manifest
        .occurrences
        .get(index)
        .and_then(|occurrence| {
            manifest
                .definitions
                .iter()
                .find(|d| d.id == occurrence.definition_id)
        })
        .and_then(|definition| definition.local_dimensions.as_ref())
    else {
        return vec![];
    };
    let placement = Affine3::from_column_major(model.occurrence_matrices_mm[index]);
    let [min, max] = match sizes {
        LocalDimensions::LocalBounds { min_mm, max_mm } => [*min_mm, *max_mm],
        LocalDimensions::AuthoredAxes { .. } => {
            let Some(bounds) = local_mesh_bounds(model, index, placement) else {
                return vec![];
            };
            bounds
        }
    };
    let size = match sizes {
        LocalDimensions::AuthoredAxes { size_mm } => *size_mm,
        LocalDimensions::LocalBounds { .. } => std::array::from_fn(|i| max[i] - min[i]),
    };
    let gap = (0..3).map(|i| max[i] - min[i]).fold(0.0, f64::max) * 0.12;
    // X along the front bottom edge, Y and Z up the right side.
    let x_end = [max[0], min[1], min[2]];
    let edges = [
        (min, x_end, Vec3::new(0.0, -gap, 0.0)),
        (x_end, [max[0], max[1], min[2]], Vec3::new(gap, 0.0, 0.0)),
        (
            x_end,
            [max[0], min[1], max[2]],
            Vec3::new(gap, -gap, 0.0) * 0.7,
        ),
    ];
    edges
        .into_iter()
        .zip(size)
        .zip(1..)
        .map(|(((from, to, offset), value), axis)| Placed {
            id: u64::MAX - axis,
            from: placement.transform_point(from.into()).into(),
            to: placement.transform_point(to.into()).into(),
            offset: placement.transform_vector(offset).into(),
            text: length_text(value, &LengthUnit::Millimetre),
        })
        .collect()
}

/// Bounds of the component's display meshes in its own axes.
fn local_mesh_bounds(model: &Model, index: usize, placement: Affine3) -> Option<[[f64; 3]; 2]> {
    let local = placement.invert().ok()?;
    let mut bounds: Option<[[f64; 3]; 2]> = None;
    for body in model.bodies.iter().filter(|body| body.occurrence == index) {
        let to_local = local.compose(Affine3::from_column_major(body.world_matrix_f64_mm));
        for bits in 0..8 {
            let corner: [f64; 3] = std::array::from_fn(|i| body.mesh_bounds_mm[(bits >> i) & 1][i]);
            let point: [f64; 3] = to_local.transform_point(corner.into()).into();
            let merged = bounds.get_or_insert([point, point]);
            for i in 0..3 {
                merged[0][i] = merged[0][i].min(point[i]);
                merged[1][i] = merged[1][i].max(point[i]);
            }
        }
    }
    bounds
}

/// `600 mm`, `60.5 cm`, `2.345 m`, `23.62 in`: the value in its display unit.
pub fn length_text(value_mm: f64, unit: &LengthUnit) -> String {
    let (value, decimals, suffix) = match unit {
        LengthUnit::Millimetre => (value_mm, 1, "mm"),
        LengthUnit::Centimetre => (value_mm / 10.0, 2, "cm"),
        LengthUnit::Metre => (value_mm / 1000.0, 3, "m"),
        LengthUnit::Inch => (value_mm / 25.4, 2, "in"),
    };
    let text = format!("{value:.decimals$}");
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.')
    } else {
        &text
    };
    format!("{text} {suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_view_format::{Dimension, InstancePath};

    fn dimension(id: u64, from: Anchor, to: Anchor, unit: LengthUnit) -> Dimension {
        Dimension {
            id,
            from,
            to,
            label_offset_mm: [0.0, 0.0, 5.0],
            value_mm: 600.0,
            display_unit: unit,
        }
    }

    fn on_second(point_mm: [f64; 3]) -> Anchor {
        Anchor {
            occurrence: Some(InstancePath {
                root_occurrence_id: 11,
                steps: vec![],
            }),
            point_mm,
        }
    }

    #[test]
    fn dimensions_follow_their_component_scene_visibility_and_section() {
        let mut package = crate::app::tests::scene_package();
        let world = Anchor {
            occurrence: None,
            point_mm: [0.0, 0.0, 0.0],
        };
        package.manifest.dimensions = vec![
            dimension(
                1,
                on_second([0.0, 0.0, 0.0]),
                on_second([10.0, 0.0, 0.0]),
                LengthUnit::Millimetre,
            ),
            dimension(2, world.clone(), world, LengthUnit::Metre),
        ];
        package.manifest.scenes[0].visible_dimensions = vec![2];
        let model = Model::from_package(package).expect("model");

        let all = placed(&model, None, &[true, true], None);
        assert_eq!(all.len(), 2);
        // The second component sits at (500, 200, 300) mm without rotation.
        assert_eq!(all[0].from, [500.0, 200.0, 300.0]);
        assert_eq!(all[0].to, [510.0, 200.0, 300.0]);
        assert_eq!(all[0].text, "600 mm", "source value, not the 10 mm span");
        assert_eq!(all[1].text, "0.6 m");

        let scene = placed(&model, Some(0), &[true, true], None);
        assert_eq!(scene.iter().map(|d| d.id).collect::<Vec<_>>(), [2]);

        let hidden = placed(&model, None, &[true, false], None);
        assert_eq!(hidden.iter().map(|d| d.id).collect::<Vec<_>>(), [2]);

        let cut = Section {
            normal: [1.0, 0.0, 0.0],
            offset_mm: 505.0,
        };
        let sectioned = placed(&model, None, &[true, true], Some(&cut));
        assert_eq!(sectioned.iter().map(|d| d.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn picked_component_shows_its_source_size_along_its_own_box() {
        let model = Model::from_package(crate::app::tests::scene_package()).expect("model");
        let overall = overall(&model, 1);
        assert_eq!(overall.len(), 3);
        assert!(
            overall.iter().all(|d| d.text == "10 mm"),
            "authored size, not a mesh measurement"
        );
        // Mesh box (5..15, 7..17, 9..19) in the component, placed at (500, 200, 300).
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-3);
        assert!(close(overall[0].from, [505.0, 207.0, 309.0]));
        assert!(close(overall[0].to, [515.0, 207.0, 309.0]));
        assert!(close(overall[1].to, [515.0, 217.0, 309.0]));
        assert!(close(overall[2].to, [515.0, 207.0, 319.0]));
        assert!(
            overall[0].offset[1] < 0.0,
            "X size sits in front of the part"
        );

        let mut package = crate::app::tests::scene_package();
        package.manifest.definitions[0].local_dimensions = None;
        let model = Model::from_package(package).expect("model");
        assert!(super::overall(&model, 1).is_empty(), "no invented size");
    }

    #[test]
    fn values_read_in_their_display_unit() {
        assert_eq!(length_text(600.0, &LengthUnit::Millimetre), "600 mm");
        assert_eq!(length_text(18.26, &LengthUnit::Millimetre), "18.3 mm");
        assert_eq!(length_text(605.0, &LengthUnit::Centimetre), "60.5 cm");
        assert_eq!(length_text(2345.0, &LengthUnit::Metre), "2.345 m");
        assert_eq!(length_text(254.0, &LengthUnit::Inch), "10 in");
    }
}
