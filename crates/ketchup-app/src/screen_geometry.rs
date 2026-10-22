//! Hit tests between points, segments and triangles on screen, in pixels.

use eframe::egui::Pos2;
use ketchup_model::tolerance::SCREEN_ROUNDING_PX;

fn screen_cross(first: Pos2, second: Pos2, third: Pos2) -> f32 {
    let left = second - first;
    let right = third - first;
    left.x * right.y - left.y * right.x
}

fn screen_point_on_segment(point: Pos2, first: Pos2, second: Pos2) -> bool {
    const EPSILON: f32 = SCREEN_ROUNDING_PX;
    screen_cross(first, second, point).abs() <= EPSILON
        && point.x >= first.x.min(second.x) - EPSILON
        && point.x <= first.x.max(second.x) + EPSILON
        && point.y >= first.y.min(second.y) - EPSILON
        && point.y <= first.y.max(second.y) + EPSILON
}

pub(crate) fn screen_segments_intersect(first: [Pos2; 2], second: [Pos2; 2]) -> bool {
    let [a, b] = first;
    let [c, d] = second;
    let ab_c = screen_cross(a, b, c);
    let ab_d = screen_cross(a, b, d);
    let cd_a = screen_cross(c, d, a);
    let cd_b = screen_cross(c, d, b);
    (ab_c.signum() != ab_d.signum() && cd_a.signum() != cd_b.signum())
        || screen_point_on_segment(c, a, b)
        || screen_point_on_segment(d, a, b)
        || screen_point_on_segment(a, c, d)
        || screen_point_on_segment(b, c, d)
}

pub(crate) fn screen_triangle_contains_point(triangle: [Pos2; 3], point: Pos2) -> bool {
    const EPSILON: f32 = SCREEN_ROUNDING_PX;
    let signs = [
        screen_cross(triangle[0], triangle[1], point),
        screen_cross(triangle[1], triangle[2], point),
        screen_cross(triangle[2], triangle[0], point),
    ];
    !signs.iter().any(|value| *value < -EPSILON) || !signs.iter().any(|value| *value > EPSILON)
}
