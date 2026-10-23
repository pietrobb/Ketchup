//! The Viewer camera: scene cameras, fit, standard views, orbit, pan, zoom,
//! projection and ray picking against the display geometry.

use crate::model::{Model, indexed_point, source_point};
use ketchup_geometry::linalg::{Affine3, Vec3};
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::{Projection, Section};
use num_traits::ToPrimitive;

/// The fixed views offered next to the saved scenes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StandardView {
    Iso,
    Front,
    Back,
    Left,
    Right,
    Top,
}

impl StandardView {
    /// In toolbar order.
    pub const ALL: [Self; 6] = [
        Self::Iso,
        Self::Front,
        Self::Back,
        Self::Left,
        Self::Right,
        Self::Top,
    ];
}

/// The Kečup editor's orbit speed, so both programs turn alike.
const ORBIT_RADIANS_PER_PX: f32 = 0.006;
/// Just short of straight up or down, where the turntable would flip.
const MAX_PITCH: f32 = 1.55;
/// The perspective lens over the shorter viewport side, about 46°: a normal
/// lens that neither flattens nor distorts a room-sized model.
pub const DEFAULT_SHORT_FOV: f32 = 0.8;

/// Where the pointer ray first meets a face: the body, which of its mesh's
/// triangle primitives and which triangle in it, and the world point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Hit {
    pub body: usize,
    pub primitive: usize,
    pub triangle: usize,
    pub point: [f64; 3],
}

/// An orbit camera around `center`; `scale` is the half span of the shorter
/// viewport side in millimetres.
#[derive(Clone, Debug)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub center: [f32; 3],
    pub scale: f32,
    radius: f32,
    roll: f32,
    short_fov: Option<f32>,
    lens_shift: [f32; 2],
}

impl Camera {
    /// Frame the whole model from the default oblique direction.
    pub fn fit(model: &Model) -> Self {
        let [min, max] = model.bounds_mm;
        let center =
            std::array::from_fn(|i| ((min[i] + max[i]) * 0.5).to_f32().expect("bounded center"));
        let radius = ((Vec3::from(max) - Vec3::from(min)).length() * 0.5)
            .to_f32()
            .expect("bounded extent")
            .max(0.001);
        Self {
            yaw: 0.7,
            pitch: 0.5,
            center,
            scale: radius * 1.25,
            radius,
            roll: 0.0,
            short_fov: None,
            lens_shift: [0.0; 2],
        }
    }

    /// The camera a scene saved, after validating it against the model.
    pub fn from_scene(
        model: &Model,
        source: &ketchup_view_format::Camera,
    ) -> Result<Self, Rejection> {
        source.validate()?;
        let mut camera = Self::fit(model);
        let direction = Vec3::from(source.eye_mm) - Vec3::from(source.target_mm);
        let distance = direction.length();
        let direction = direction / distance;
        let d: [f64; 3] = direction.into();
        camera.yaw = d[1].atan2(d[0]).to_f32().expect("angle");
        camera.pitch = d[2].clamp(-1.0, 1.0).asin().to_f32().expect("angle");
        camera.center = source
            .target_mm
            .map(|v| v.to_f32().expect("bounded target"));
        let (_, right, up) = camera.basis();
        let saved_up = Vec3::from(source.up);
        camera.roll = (-saved_up.dot(Vec3::from(right.map(f64::from))))
            .atan2(saved_up.dot(Vec3::from(up.map(f64::from))))
            .to_f32()
            .expect("roll");
        // Depth range includes distant geometry even when the author targets empty space.
        for bits in 0..8 {
            let point = std::array::from_fn(|i| model.bounds_mm[(bits >> i) & 1][i]);
            camera.radius = camera.radius.max(
                (Vec3::from(point) - Vec3::from(source.target_mm))
                    .length()
                    .to_f32()
                    .expect("bounded scene extent"),
            );
        }
        match source.projection {
            Projection::Orthographic { short_span_mm } => {
                camera.scale = (short_span_mm * 0.5).to_f32().expect("bounded span");
            }
            Projection::Perspective {
                short_fov_radians,
                lens_shift_short,
            } => {
                camera.short_fov = Some(short_fov_radians.to_f32().expect("bounded FOV"));
                camera.scale = (distance * (short_fov_radians * 0.5).tan())
                    .to_f32()
                    .expect("bounded scale");
                camera.lens_shift = lens_shift_short.map(|v| v.to_f32().expect("bounded shift"));
            }
        }
        // not a tolerance: smallest span whose f32 projection matrix stays finite.
        if !camera.scale.is_finite() || camera.scale < 1e-6 {
            return Err(
                Rejection::new("viewer.camera.range", RejectionPhase::Request)
                    .target("scene camera")
                    .reason("The scene camera span is too small for finite GPU projection.")
                    .fix_hint(
                        "Re-export the scene with a larger camera span or eye-to-target distance.",
                    ),
            );
        }
        Ok(camera)
    }

    /// Turntable orbit around the center by a pointer delta in pixels, as in
    /// SketchUp and the Kečup editor: dragging right turns the model right
    /// about the vertical axis, dragging down tips its top towards the viewer.
    /// Z stays up and the view never flips over the pole.
    pub fn orbit(&mut self, delta: [f32; 2]) {
        if !delta.iter().all(|v| v.is_finite()) {
            return;
        }
        self.yaw = (self.yaw - delta[0] * ORBIT_RADIANS_PER_PX) % std::f32::consts::TAU;
        self.pitch = (self.pitch + delta[1] * ORBIT_RADIANS_PER_PX).clamp(-MAX_PITCH, MAX_PITCH);
        // A tilted scene camera levels out once the customer turns the model.
        self.roll = 0.0;
    }

    /// Zoom by `factor` (> 1 zooms in), limited relative to the model size.
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.scale = (self.scale / factor).clamp(self.radius * 0.01, self.radius * 100.0);
        }
    }

    /// Zoom by `factor` keeping the point under the pointer at `ndc` in place,
    /// like the mouse wheel in SketchUp; the orbit center moves along.
    pub fn zoom_at(&mut self, factor: f32, ndc: [f32; 2], aspect: f32) {
        let before = self.scale;
        self.zoom(factor);
        let kept = 1.0 - self.scale / before;
        if !(kept.is_finite() && ndc.iter().all(|v| v.is_finite()) && aspect > 0.0) {
            return;
        }
        let (_, right, up) = self.basis();
        let sx = aspect.max(1.0);
        let sy = (1.0 / aspect).max(1.0);
        let shift = if self.short_fov.is_some() {
            [2.0 * self.lens_shift[0] / sx, 2.0 * self.lens_shift[1] / sy]
        } else {
            [0.0; 2]
        };
        // Offset of the pointed point from the center on the target plane.
        let offset = [
            (ndc[0] - shift[0]) * before * sx,
            (ndc[1] - shift[1]) * before * sy,
        ];
        for i in 0..3 {
            self.center[i] = (self.center[i] + (right[i] * offset[0] + up[i] * offset[1]) * kept)
                .clamp(-1e9, 1e9);
        }
    }

    /// Moves the view with the pointer: `delta` in pixels of a viewport whose
    /// shorter side is `short_px`, so the model follows the finger 1:1.
    pub fn pan(&mut self, delta: [f32; 2], short_px: f32) {
        if !(short_px.is_finite() && short_px > 0.0 && delta.iter().all(|v| v.is_finite())) {
            return;
        }
        let (_, right, up) = self.basis();
        let mm_per_px = 2.0 * self.scale / short_px;
        for i in 0..3 {
            // Screen y grows downwards; the package coordinate limit bounds the target.
            self.center[i] = (self.center[i]
                - (right[i] * delta[0] - up[i] * delta[1]) * mm_per_px)
                .clamp(-1e9, 1e9);
        }
    }

    /// A standard direction framed on the whole model, keeping the projection.
    pub fn standard(&mut self, model: &Model, view: StandardView) {
        let fitted = Self::fit(model);
        let (yaw, pitch) = match view {
            StandardView::Iso => (-std::f32::consts::FRAC_PI_4, 0.615_479_7),
            StandardView::Front => (-std::f32::consts::FRAC_PI_2, 0.0),
            StandardView::Back => (std::f32::consts::FRAC_PI_2, 0.0),
            StandardView::Left => (std::f32::consts::PI, 0.0),
            StandardView::Right => (0.0, 0.0),
            StandardView::Top => (-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2),
        };
        self.yaw = yaw;
        self.pitch = pitch;
        self.roll = 0.0;
        self.lens_shift = [0.0; 2];
        self.center = fitted.center;
        self.radius = fitted.radius;
        self.scale = fitted.scale;
    }

    /// Whether the view is in perspective.
    pub fn is_perspective(&self) -> bool {
        self.short_fov.is_some()
    }

    /// Perspective with [`DEFAULT_SHORT_FOV`] (`true`) or orthographic. The
    /// plane through the center facing the viewer keeps what it shows, so
    /// the view does not jump: same center, same span, and a scene's lens
    /// shift becomes a move of the center when the shift goes away.
    pub fn set_perspective(&mut self, perspective: bool) {
        if perspective == self.is_perspective() {
            return;
        }
        if perspective {
            self.short_fov = Some(DEFAULT_SHORT_FOV);
            return;
        }
        let (_, right, up) = self.basis();
        for i in 0..3 {
            self.center[i] -=
                2.0 * self.scale * (right[i] * self.lens_shift[0] + up[i] * self.lens_shift[1]);
        }
        self.lens_shift = [0.0; 2];
        self.short_fov = None;
    }

    /// View direction, screen right and screen up, with roll applied.
    fn basis(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let direction = [
            self.yaw.cos() * self.pitch.cos(),
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
        ];
        let right = [-self.yaw.sin(), self.yaw.cos(), 0.0];
        let up = [
            -self.yaw.cos() * self.pitch.sin(),
            -self.yaw.sin() * self.pitch.sin(),
            self.pitch.cos(),
        ];
        (
            direction,
            std::array::from_fn(|i| right[i] * self.roll.cos() + up[i] * self.roll.sin()),
            std::array::from_fn(|i| up[i] * self.roll.cos() - right[i] * self.roll.sin()),
        )
    }

    /// View-projection matrix for a viewport of width/height `aspect`.
    pub fn matrix(&self, aspect: f32) -> [[f32; 4]; 4] {
        let (direction, right, up) = self.basis();
        let sx = aspect.max(1.0);
        let sy = (1.0 / aspect).max(1.0);
        if let Some(fov) = self.short_fov {
            let tangent = (fov * 0.5).tan();
            let distance = self.scale / tangent;
            let near = (self.radius * 0.001).min(distance * 0.5).max(0.000001);
            let far = distance + self.radius * 4.0;
            let a = far / (far - near);
            let b = near * a;
            let w = [
                -direction[0],
                -direction[1],
                -direction[2],
                distance + projected_coordinate(direction, self.center),
            ];
            let shift = [2.0 * self.lens_shift[0] / sx, 2.0 * self.lens_shift[1] / sy];
            let x = std::array::from_fn::<_, 3, _>(|i| right[i] / (tangent * sx));
            let y = std::array::from_fn::<_, 3, _>(|i| up[i] / (tangent * sy));
            std::array::from_fn(|col| {
                let px = if col < 3 {
                    x[col]
                } else {
                    -projected_coordinate(x, self.center)
                };
                let py = if col < 3 {
                    y[col]
                } else {
                    -projected_coordinate(y, self.center)
                };
                [
                    px + shift[0] * w[col],
                    py + shift[1] * w[col],
                    a * w[col] - if col == 3 { b } else { 0.0 },
                    w[col],
                ]
            })
        } else {
            let x = right.map(|v| v / (self.scale * sx));
            let y = up.map(|v| v / (self.scale * sy));
            let z = direction.map(|v| -v / (self.radius * 8.0));
            [
                [x[0], y[0], z[0], 0.0],
                [x[1], y[1], z[1], 0.0],
                [x[2], y[2], z[2], 0.0],
                [
                    -projected_coordinate(x, self.center),
                    -projected_coordinate(y, self.center),
                    0.5 - projected_coordinate(z, self.center),
                    1.0,
                ],
            ]
        }
    }

    /// A source point in normalized device coordinates.
    pub fn project(&self, point: [f32; 3], aspect: f32) -> [f32; 3] {
        let m = self.matrix(aspect);
        let clip: [f32; 4] = std::array::from_fn(|i| {
            projected_coordinate([m[0][i], m[1][i], m[2][i]], point) + m[3][i]
        });
        [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]]
    }

    /// The visible occurrence whose faces the ray through `ndc` hits first,
    /// ignoring geometry removed by `section`.
    pub fn pick(
        &self,
        model: &Model,
        ndc: [f32; 2],
        aspect: f32,
        visible: &[bool],
        section: Option<&Section>,
    ) -> Option<usize> {
        self.pick_point(model, ndc, aspect, visible, section)
            .map(|(index, _)| index)
    }

    /// Like [`Camera::pick`], with the world point hit on that occurrence.
    pub fn pick_point(
        &self,
        model: &Model,
        ndc: [f32; 2],
        aspect: f32,
        visible: &[bool],
        section: Option<&Section>,
    ) -> Option<(usize, [f64; 3])> {
        self.pick_counted(model, ndc, aspect, visible, section).0
    }

    /// [`Camera::pick_point`] and how many triangles it tested: only bodies
    /// whose bounds the ray crosses, from the buffers kept when the package
    /// was opened.
    fn pick_counted(
        &self,
        model: &Model,
        ndc: [f32; 2],
        aspect: f32,
        visible: &[bool],
        section: Option<&Section>,
    ) -> (Option<(usize, [f64; 3])>, usize) {
        let (hit, tested) = self.hit_counted(model, ndc, aspect, visible, section);
        (
            hit.map(|hit| (model.bodies[hit.body].occurrence, hit.point)),
            tested,
        )
    }

    /// The first triangle the ray through `ndc` meets, as [`Camera::pick_point`].
    pub(crate) fn pick_hit(
        &self,
        model: &Model,
        ndc: [f32; 2],
        aspect: f32,
        visible: &[bool],
        section: Option<&Section>,
    ) -> Option<Hit> {
        self.hit_counted(model, ndc, aspect, visible, section).0
    }

    /// The pointer ray through `ndc`: its origin and direction, towards the scene.
    pub(crate) fn ray(&self, ndc: [f32; 2], aspect: f32) -> (Vec3, Vec3) {
        let (direction, right, up) = self.basis();
        let direction = Vec3::from(direction.map(f64::from));
        let right = Vec3::from(right.map(f64::from));
        let up = Vec3::from(up.map(f64::from));
        let center = Vec3::from(self.center.map(f64::from));
        let sx = f64::from(aspect.max(1.0));
        let sy = f64::from((1.0 / aspect).max(1.0));
        if let Some(fov) = self.short_fov {
            let tangent = f64::from((fov * 0.5).tan());
            (
                center + direction * f64::from(self.scale) / tangent,
                right * ((f64::from(ndc[0]) * sx - 2.0 * f64::from(self.lens_shift[0])) * tangent)
                    + up * ((f64::from(ndc[1]) * sy - 2.0 * f64::from(self.lens_shift[1]))
                        * tangent)
                    - direction,
            )
        } else {
            (
                center
                    + right * (f64::from(ndc[0] * self.scale) * sx)
                    + up * (f64::from(ndc[1] * self.scale) * sy)
                    + direction * f64::from(self.radius * 4.0),
                -direction,
            )
        }
    }

    /// [`Camera::pick_hit`] and how many triangles it tested: only bodies
    /// whose bounds the ray crosses.
    fn hit_counted(
        &self,
        model: &Model,
        ndc: [f32; 2],
        aspect: f32,
        visible: &[bool],
        section: Option<&Section>,
    ) -> (Option<Hit>, usize) {
        let (origin, ray) = self.ray(ndc, aspect);
        let mut nearest: Option<(Hit, f32)> = None;
        let mut tested = 0;
        for (body_index, body) in model.bodies.iter().enumerate() {
            if !visible[body.occurrence] || !ray_crosses(origin, ray, body.bounds_mm) {
                continue;
            }
            let placement = Affine3::from_column_major(body.world_matrix_mm.map(f64::from));
            for (primitive, (positions, indices)) in model.triangle_buffers(body.mesh).enumerate() {
                for triangle in 0..indices.len() / 12 {
                    tested += 1;
                    let points: [[f32; 3]; 3] = std::array::from_fn(|i| {
                        let point = indexed_point(positions, indices, triangle * 3 + i);
                        let world: [f64; 3] = placement
                            .transform_point(Vec3::from(source_point(point)))
                            .into();
                        world.map(|v| v.to_f32().expect("bounded world point"))
                    });
                    if let Some(hit) = ray_hit(origin, ray, points) {
                        let coordinates: [f64; 3] = hit.into();
                        let depth = self.project(
                            coordinates.map(|v| v.to_f32().expect("bounded hit")),
                            aspect,
                        )[2];
                        if !(0.0..=1.0).contains(&depth) {
                            continue;
                        }
                        if section.is_some_and(|s| Vec3::from(s.normal).dot(hit) > s.offset_mm) {
                            continue;
                        }
                        if nearest.is_none_or(|(_, z)| depth < z) {
                            let hit = Hit {
                                body: body_index,
                                primitive,
                                triangle,
                                point: coordinates,
                            };
                            nearest = Some((hit, depth));
                        }
                    }
                }
            }
        }
        (nearest.map(|(hit, _)| hit), tested)
    }

    /// The point on the plane through the orbit center facing the viewer
    /// that `ndc` shows; where an arrow's free end goes.
    pub fn point_on_view_plane(&self, ndc: [f32; 2], aspect: f32) -> [f64; 3] {
        let (_, right, up) = self.basis();
        let sx = aspect.max(1.0);
        let sy = (1.0 / aspect).max(1.0);
        let shift = if self.short_fov.is_some() {
            [2.0 * self.lens_shift[0] / sx, 2.0 * self.lens_shift[1] / sy]
        } else {
            [0.0; 2]
        };
        let offset = [
            (ndc[0] - shift[0]) * self.scale * sx,
            (ndc[1] - shift[1]) * self.scale * sy,
        ];
        std::array::from_fn(|i| {
            f64::from(self.center[i] + right[i] * offset[0] + up[i] * offset[1])
        })
    }

    /// This view as a package camera, so a comment restores it elsewhere.
    pub fn to_scene(&self) -> ketchup_view_format::Camera {
        let (direction, _, up) = self.basis();
        let target = self.center.map(f64::from);
        let (distance, projection) = match self.short_fov {
            Some(fov) => (
                f64::from(self.scale / (fov * 0.5).tan()),
                Projection::Perspective {
                    short_fov_radians: f64::from(fov),
                    lens_shift_short: self.lens_shift.map(f64::from),
                },
            ),
            None => (
                f64::from(self.radius * 2.0),
                Projection::Orthographic {
                    short_span_mm: f64::from(self.scale * 2.0),
                },
            ),
        };
        ketchup_view_format::Camera {
            eye_mm: std::array::from_fn(|i| target[i] + f64::from(direction[i]) * distance),
            target_mm: target,
            up: up.map(f64::from),
            projection,
        }
    }
}

/// Whether the ray from `origin` along `ray` passes through `bounds`, padded
/// for the single-precision points the triangles are tested with.
fn ray_crosses(origin: Vec3, ray: Vec3, bounds: [[f64; 3]; 2]) -> bool {
    let origin: [f64; 3] = origin.into();
    let ray: [f64; 3] = ray.into();
    let scale = bounds.iter().flatten().fold(1.0_f64, |m, v| m.max(v.abs()));
    let pad = scale * f64::from(f32::EPSILON) * 16.0;
    let (mut near, mut far) = (0.0_f64, f64::INFINITY);
    for i in 0..3 {
        let (min, max) = (bounds[0][i] - pad, bounds[1][i] + pad);
        if ray[i] == 0.0 {
            if origin[i] < min || origin[i] > max {
                return false;
            }
            continue;
        }
        let (a, b) = ((min - origin[i]) / ray[i], (max - origin[i]) / ray[i]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    near <= far
}

/// Möller–Trumbore ray/triangle intersection point.
fn ray_hit(origin: Vec3, ray: Vec3, points: [[f32; 3]; 3]) -> Option<Vec3> {
    let [a, b, c] = points.map(|p| Vec3::from(p.map(f64::from)));
    let edge1 = b - a;
    let edge2 = c - a;
    let p = ray.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs()
        <= edge1.length() * edge2.length() * ray.length() * ketchup_tolerance::NEGLIGIBLE
    {
        return None;
    }
    let offset = origin - a;
    let u = offset.dot(p) / determinant;
    let q = offset.cross(edge1);
    let v = ray.dot(q) / determinant;
    let distance = edge2.dot(q) / determinant;
    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && distance >= 0.0).then_some(origin + ray * distance)
}

fn projected_coordinate(a: [f32; 3], b: [f32; 3]) -> f32 {
    Vec3::from(a.map(f64::from))
        .dot(Vec3::from(b.map(f64::from)))
        .to_f32()
        .expect("bounded camera coordinate")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::package;

    #[test]
    fn perspective_pick_matches_world_section_and_rejects_unrenderable_span() {
        let package = crate::app::tests::scene_package();
        let model = Model::from_package(package).expect("model");
        let source = ketchup_view_format::Camera {
            projection: Projection::Perspective {
                short_fov_radians: 0.5,
                lens_shift_short: [0.2, -0.1],
            },
            ..model.manifest().scenes[0].camera.clone()
        };
        let camera = Camera::from_scene(&model, &source).expect("camera");
        for aspect in [0.5, 1.0, 2.0] {
            let point = camera.project([88.0, 208.0, 314.0], aspect);
            let ndc = [point[0], point[1]];
            assert_eq!(
                camera.pick(&model, ndc, aspect, &[true, false], None),
                Some(0)
            );
            let cut = Section {
                normal: [1.0, 0.0, 0.0],
                offset_mm: 87.0,
            };
            assert_eq!(
                camera.pick(&model, ndc, aspect, &[true, false], Some(&cut)),
                None
            );
            let cut = Section {
                offset_mm: 89.0,
                ..cut
            };
            assert_eq!(
                camera.pick(&model, ndc, aspect, &[true, false], Some(&cut)),
                Some(0)
            );
            assert_eq!(camera.pick(&model, ndc, aspect, &[false, true], None), None);
        }
        let source = ketchup_view_format::Camera {
            projection: Projection::Orthographic {
                short_span_mm: 1e-300,
            },
            ..source
        };
        assert!(Camera::from_scene(&model, &source).is_err());
    }

    #[test]
    fn switching_the_projection_keeps_the_view_and_picking_works_in_both() {
        let model = Model::from_package(crate::app::tests::scene_package()).expect("model");
        let shifted = ketchup_view_format::Camera {
            projection: Projection::Perspective {
                short_fov_radians: 0.5,
                lens_shift_short: [0.2, -0.1],
            },
            ..model.manifest().scenes[0].camera.clone()
        };
        let fitted = Camera::fit(&model);
        for start in [
            fitted,
            Camera::from_scene(&model, &shifted).expect("camera"),
        ] {
            let mut camera = start.clone();
            for perspective in [!start.is_perspective(), start.is_perspective()] {
                let before = camera.clone();
                camera.set_perspective(perspective);
                assert_eq!(camera.is_perspective(), perspective);
                assert_eq!(camera.scale, before.scale, "same span");
                for aspect in [0.5, 1.0, 2.0] {
                    // Points on the plane through the center stay where they were.
                    for ndc in [[0.0, 0.0], [0.6, -0.4], [-0.3, 0.5]] {
                        let point = before.point_on_view_plane(ndc, aspect);
                        let shown =
                            camera.project(point.map(|v| v.to_f32().expect("point")), aspect);
                        assert!(
                            (shown[0] - ndc[0]).abs() < 1e-4 && (shown[1] - ndc[1]).abs() < 1e-4,
                            "{perspective}: {ndc:?} -> {shown:?}"
                        );
                    }
                    // What the previous projection picked is picked again, at the same point.
                    let at = before.project([88.0, 208.0, 314.0], aspect);
                    let (_, seen) = before
                        .pick_point(&model, [at[0], at[1]], aspect, &[true, false], None)
                        .expect("pick before");
                    let at = camera.project(seen.map(|v| v.to_f32().expect("hit")), aspect);
                    let (picked, hit) = camera
                        .pick_point(&model, [at[0], at[1]], aspect, &[true, false], None)
                        .expect("pick after");
                    assert_eq!(picked, 0);
                    assert!(
                        (Vec3::from(hit) - Vec3::from(seen)).length() < 0.01,
                        "{perspective}: picked {hit:?}, before {seen:?}"
                    );
                }
            }
        }
        let mut camera = Camera::fit(&model);
        camera.set_perspective(true);
        assert_eq!(camera.short_fov, Some(DEFAULT_SHORT_FOV));
        camera.standard(&model, StandardView::Front);
        assert!(
            camera.is_perspective(),
            "standard views keep the projection"
        );
    }

    #[test]
    fn a_click_tests_only_the_triangles_of_bodies_under_the_pointer() {
        let model = Model::from_package(crate::app::tests::scene_package()).expect("model");
        let camera =
            Camera::from_scene(&model, &model.manifest().scenes[0].camera).expect("camera");
        let both = [true, true];
        let point = camera.project([88.0, 208.0, 314.0], 1.0);
        assert_eq!(
            camera.pick_counted(&model, [point[0], point[1]], 1.0, &both, None),
            (
                camera.pick_point(&model, [point[0], point[1]], 1.0, &both, None),
                1
            )
        );
        assert_eq!(
            camera.pick(&model, [point[0], point[1]], 1.0, &both, None),
            Some(0)
        );
        let beside = camera.project([88.0, 1000.0, 314.0], 1.0);
        assert_eq!(
            camera.pick_counted(&model, [beside[0], beside[1]], 1.0, &both, None),
            (None, 0)
        );
    }

    #[test]
    fn cameras_keep_short_axis_scene_span_and_perspective_shift() {
        let model = Model::from_package(package(0.1)).expect("fixture");
        let mut camera = Camera::fit(&model);
        for aspect in [0.5, 1.0, 2.0] {
            assert_eq!(camera.project(camera.center, aspect), [0.0, 0.0, 0.5]);
        }
        let source = ketchup_view_format::Camera {
            eye_mm: [100.0, 0.0, 0.0],
            target_mm: [0.0; 3],
            up: [0.0, 0.0, 1.0],
            projection: Projection::Orthographic {
                short_span_mm: 40.0,
            },
        };
        camera = Camera::from_scene(&model, &source).expect("scene camera");
        assert!((camera.project([0.0, 20.0, 0.0], 2.0)[0] - 0.5).abs() < 1e-6);
        assert!((camera.project([0.0, 0.0, 20.0], 0.5)[1] - 0.5).abs() < 1e-6);
        let perspective = ketchup_view_format::Camera {
            projection: Projection::Perspective {
                short_fov_radians: std::f64::consts::FRAC_PI_2,
                lens_shift_short: [0.1, -0.2],
            },
            ..source
        };
        camera = Camera::from_scene(&model, &perspective).expect("scene camera");
        assert!((camera.project([0.0; 3], 2.0)[0] - 0.1).abs() < 1e-6);
        assert!((camera.project([0.0; 3], 0.5)[1] + 0.2).abs() < 1e-6);
        let before = camera.project([0.0, 10.0, 10.0], 1.0);
        camera.orbit([30.0, 20.0]);
        assert_ne!(before, camera.project([0.0, 10.0, 10.0], 1.0));
        let scale = camera.scale;
        camera.zoom(2.0);
        assert_eq!(camera.scale, scale / 2.0);
        camera.zoom(f32::NAN);
        assert_eq!(camera.scale, scale / 2.0);
    }

    #[test]
    fn orbit_turns_like_sketchup_and_wheel_zoom_keeps_the_pointed_point() {
        let model = Model::from_package(package(0.1)).expect("fixture");
        let mut camera = Camera::fit(&model);
        camera.standard(&model, StandardView::Front);
        // A point on the front-right of the center, seen from the front.
        let point = std::array::from_fn(|i| camera.center[i] + [10.0, -10.0, 0.0][i]);
        let x = camera.project(point, 1.0)[0];
        camera.orbit([40.0, 0.0]);
        assert!(
            camera.project(point, 1.0)[0] > x,
            "dragging right turns the model right: its near side follows the pointer"
        );
        let top = std::array::from_fn(|i| camera.center[i] + [0.0, 0.0, 10.0][i]);
        let depth = camera.project(top, 1.0)[2];
        camera.orbit([0.0, 40.0]);
        assert!(
            camera.project(top, 1.0)[2] < depth,
            "dragging down raises the eye, the top tips towards the viewer"
        );
        for _ in 0..100 {
            camera.orbit([0.0, 500.0]);
        }
        let (_, _, up) = camera.basis();
        assert!(
            camera.pitch <= MAX_PITCH && up[2] > 0.0,
            "never over the pole"
        );
        camera.roll = 0.3;
        camera.orbit([1.0, 0.0]);
        assert_eq!(camera.roll, 0.0, "orbit keeps Z up");

        for perspective in [false, true] {
            let mut camera = Camera::fit(&model);
            if perspective {
                camera.short_fov = Some(0.8);
                camera.lens_shift = [0.1, -0.05];
            }
            for aspect in [0.5, 1.6] {
                let pointed = [0.4, -0.3];
                let (_, right, up) = camera.basis();
                let shift = camera.project(camera.center, aspect);
                let sx = aspect.max(1.0);
                let sy = (1.0 / aspect).max(1.0);
                let world: [f32; 3] = std::array::from_fn(|i| {
                    camera.center[i]
                        + right[i] * (pointed[0] - shift[0]) * camera.scale * sx
                        + up[i] * (pointed[1] - shift[1]) * camera.scale * sy
                });
                let before = camera.project(world, aspect);
                camera.zoom_at(1.7, pointed, aspect);
                let after = camera.project(world, aspect);
                assert!(
                    (before[0] - pointed[0]).abs() < 1e-4
                        && (after[0] - before[0]).abs() < 1e-4
                        && (after[1] - before[1]).abs() < 1e-4,
                    "the point under the pointer stays put: {before:?} -> {after:?}"
                );
                camera.zoom_at(1.0 / 1.7, pointed, aspect);
            }
        }
    }
}
