//! Orientation guides drawn under the model: the red/green/blue axes through
//! the source origin, dashed on their negative side, and a ground grid on
//! z = 0 sized to the model footprint. Display only, never part of a package.

use num_traits::ToPrimitive;

/// Line segments in source millimetres, grouped by how they are coloured.
pub struct Guides {
    /// Positive X, Y and Z axes, in that order.
    pub axes: [Vec<[[f32; 3]; 2]>; 3],
    /// Dashed negative X, Y and Z axes.
    pub negative_axes: [Vec<[[f32; 3]; 2]>; 3],
    pub grid: Vec<[[f32; 3]; 2]>,
    /// Distance between grid lines in millimetres.
    pub step_mm: f64,
}

/// At most this many grid lines per direction, whatever the model size.
const MAX_GRID_LINES: f64 = 100.0;
const DASHES: usize = 40;

impl Guides {
    /// Guides framing a model whose source bounds are `bounds_mm`.
    pub fn new(bounds_mm: [[f64; 3]; 2]) -> Self {
        let [min, max] = bounds_mm;
        let footprint = (max[0] - min[0]).max(max[1] - min[1]).max(1.0);
        let step_mm = nice_step(footprint / 10.0);
        let margin = footprint * 0.25;
        // The grid covers the footprint and, unless far away, the origin of the axes.
        let near_origin =
            (0..2).all(|i| min[i] - footprint * 2.0 <= 0.0 && max[i] + footprint * 2.0 >= 0.0);
        let (low, high) = if near_origin {
            (
                [min[0].min(0.0), min[1].min(0.0)],
                [max[0].max(0.0), max[1].max(0.0)],
            )
        } else {
            ([min[0], min[1]], [max[0], max[1]])
        };
        let lower = [0, 1].map(|i| ((low[i] - margin) / step_mm).floor());
        let upper = [0, 1].map(|i| ((high[i] + margin) / step_mm).ceil());
        let lines = [0, 1].map(|i| (upper[i] - lower[i]).min(MAX_GRID_LINES * 2.0));
        let mut grid = Vec::new();
        for axis in 0..2 {
            let other = 1 - axis;
            let from = lower[other] * step_mm;
            let to = (lower[other] + lines[other]) * step_mm;
            for k in 0..=lines[axis].to_usize().unwrap_or(0) {
                let at = (lower[axis] + k.to_f64().unwrap_or(0.0)) * step_mm;
                let mut a = [0.0; 3];
                let mut b = [0.0; 3];
                a[axis] = at;
                b[axis] = at;
                a[other] = from;
                b[other] = to;
                grid.push([point(a), point(b)]);
            }
        }
        let reach = (0..3)
            .map(|i| min[i].abs().max(max[i].abs()))
            .fold(footprint, f64::max)
            * 1.25;
        let axes = std::array::from_fn(|i| {
            let mut end = [0.0; 3];
            end[i] = reach;
            vec![[point([0.0; 3]), point(end)]]
        });
        let negative_axes = std::array::from_fn(|i| {
            (0..DASHES)
                .step_by(2)
                .map(|k| {
                    let dash = reach / DASHES.to_f64().unwrap_or(1.0);
                    let mut a = [0.0; 3];
                    let mut b = [0.0; 3];
                    a[i] = -dash * k.to_f64().unwrap_or(0.0);
                    b[i] = a[i] - dash;
                    [point(a), point(b)]
                })
                .collect()
        });
        Self {
            axes,
            negative_axes,
            grid,
            step_mm,
        }
    }
}

/// 1, 2 or 5 × 10ⁿ millimetres, the first not below `raw`.
fn nice_step(raw: f64) -> f64 {
    let magnitude = 10f64.powf(raw.max(1.0).log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * magnitude)
        .find(|step| *step >= raw)
        .unwrap_or(10.0 * magnitude)
}

fn point(source_mm: [f64; 3]) -> [f32; 3] {
    source_mm.map(|v| v.to_f32().expect("bounded guide point"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_is_ground_level_round_and_covers_footprint_and_origin() {
        let guides = Guides::new([[2000.0, 500.0, 0.0], [6000.0, 3500.0, 2400.0]]);
        assert_eq!(guides.step_mm, 500.0, "4 m footprint -> 0.5 m grid");
        assert!(
            guides
                .grid
                .iter()
                .flatten()
                .all(|p| p[2] == 0.0 && p[0] % 500.0 == 0.0 && p[1] % 500.0 == 0.0),
            "grid lies on z = 0 at whole steps"
        );
        let covered = |axis: usize, value: f32| {
            let values = guides.grid.iter().flatten().map(|p| p[axis]);
            values.clone().fold(f32::MAX, f32::min) <= value
                && values.fold(f32::MIN, f32::max) >= value
        };
        assert!(covered(0, 0.0) && covered(0, 6000.0) && covered(1, 0.0) && covered(1, 3500.0));
        assert_eq!(guides.axes[2][0], [[0.0; 3], [0.0, 0.0, 7500.0]]);
        assert!(
            guides.negative_axes[0]
                .iter()
                .flatten()
                .all(|p| p[0] <= 0.0)
        );
    }

    #[test]
    fn huge_or_tiny_models_keep_a_bounded_grid() {
        for bounds in [
            [[-1e8, -1e8, 0.0], [1e8, 1e8, 1.0]],
            [[0.0, 0.0, 0.0], [0.1, 0.1, 0.1]],
            [[1e7, 1e7, 0.0], [1e7 + 10.0, 1e7 + 10.0, 10.0]],
        ] {
            let guides = Guides::new(bounds);
            assert!(guides.grid.len() <= 2 * (2 * MAX_GRID_LINES as usize + 1));
            assert!(
                guides
                    .grid
                    .iter()
                    .flatten()
                    .flatten()
                    .all(|v| v.is_finite())
            );
        }
        let far = Guides::new([[1e7, 1e7, 0.0], [1e7 + 10.0, 1e7 + 10.0, 10.0]]);
        assert!(
            far.grid.iter().flatten().any(|p| p[0] >= 1e7 + 10.0)
                && far.grid.iter().flatten().all(|p| p[0] >= 1e7 - 10.0),
            "a model far from the origin gets its grid under the model"
        );
        assert_eq!(nice_step(0.3), 1.0);
        assert_eq!(nice_step(130.0), 200.0);
        assert_eq!(nice_step(4000.0), 5000.0);
    }
}
