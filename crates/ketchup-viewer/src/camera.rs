//! The Viewer camera: scene cameras, fit, standard views, orbit, pan, zoom,
//! projection and ray picking against the display geometry.

use crate::model::{Model, source_point};
use ketchup_geometry::linalg::{Affine3, Vec3};
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::{PrimitiveTopology, Projection, Section};
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

    /// Rotate around the center by a pointer delta in pixels.
    pub fn orbit(&mut self, delta: [f32; 2]) {
        self.yaw = (self.yaw + delta[0] * 0.01) % std::f32::consts::TAU;
        self.pitch = (self.pitch + delta[1] * 0.01).clamp(-1.5, 1.5);
    }

    /// Zoom by `factor` (> 1 zooms in), limited relative to the model size.
    pub fn zoom(&mut self, factor: f32) {
        if factor.is_finite() && factor > 0.0 {
            self.scale = (self.scale / factor).clamp(self.radius * 0.01, self.radius * 100.0);
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
    ) -> Result<Option<usize>, Rejection> {
        let geometry = model.geometry()?;
        let (direction, right, up) = self.basis();
        let direction = Vec3::from(direction.map(f64::from));
        let right = Vec3::from(right.map(f64::from));
        let up = Vec3::from(up.map(f64::from));
        let center = Vec3::from(self.center.map(f64::from));
        let sx = f64::from(aspect.max(1.0));
        let sy = f64::from((1.0 / aspect).max(1.0));
        let (origin, ray) = if let Some(fov) = self.short_fov {
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
        };
        let mut nearest: Option<(usize, f32)> = None;
        for body in &model.bodies {
            if !visible[body.occurrence] {
                continue;
            }
            let placement = Affine3::from_column_major(body.world_matrix_mm.map(f64::from));
            for primitive in &geometry.meshes[body.mesh] {
                if primitive.topology != PrimitiveTopology::Triangles {
                    continue;
                }
                for triangle in primitive.index_bytes().chunks_exact(12) {
                    let points: [[f32; 3]; 3] = std::array::from_fn(|i| {
                        let index = u32::from_le_bytes(
                            triangle[i * 4..i * 4 + 4].try_into().expect("index"),
                        );
                        let offset = usize::try_from(index).expect("bounded index") * 12;
                        let point = std::array::from_fn(|k| {
                            f32::from_le_bytes(
                                primitive.position_bytes()[offset + k * 4..offset + k * 4 + 4]
                                    .try_into()
                                    .expect("point"),
                            )
                        });
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
                            nearest = Some((body.occurrence, depth));
                        }
                    }
                }
            }
        }
        Ok(nearest.map(|(index, _)| index))
    }
}

/// Möller–Trumbore ray/triangle intersection point.
fn ray_hit(origin: Vec3, ray: Vec3, points: [[f32; 3]; 3]) -> Option<Vec3> {
    let [a, b, c] = points.map(|p| Vec3::from(p.map(f64::from)));
    let edge1 = b - a;
    let edge2 = c - a;
    let p = ray.cross(edge2);
    let determinant = edge1.dot(p);
    if determinant.abs() <= edge1.length() * edge2.length() * ray.length() * 1e-12 {
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
                camera
                    .pick(&model, ndc, aspect, &[true, false], None)
                    .expect("pick"),
                Some(0)
            );
            let cut = Section {
                normal: [1.0, 0.0, 0.0],
                offset_mm: 87.0,
            };
            assert_eq!(
                camera
                    .pick(&model, ndc, aspect, &[true, false], Some(&cut))
                    .expect("pick"),
                None
            );
            let cut = Section {
                offset_mm: 89.0,
                ..cut
            };
            assert_eq!(
                camera
                    .pick(&model, ndc, aspect, &[true, false], Some(&cut))
                    .expect("pick"),
                Some(0)
            );
            assert_eq!(
                camera
                    .pick(&model, ndc, aspect, &[false, true], None)
                    .expect("pick"),
                None
            );
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
}
