//! The CPU renderer behind [`super::render_presentation`]. Only `f64` arithmetic and square
//! roots, no trigonometry from the platform's maths library, so the pixels are the same on
//! every system.

use super::{
    Background, PresentationError, PresentationImage, PresentationOptions, PresentationScene,
};
use ketchup_geometry::linalg::Vec3;
use std::f64::consts::FRAC_1_SQRT_2;

/// tan(15°): a 30° vertical field of view, a natural lens for architecture and furniture.
const TAN_HALF_FOV: f64 = 0.267_949_192_431_122_7;
/// Share of the image the model may fill along its tighter side.
const FRAME_FILL: f64 = 0.88;
const SHADOW_MAP_SIDE: usize = 2048;
const NO_TRIANGLE: u32 = u32::MAX;
const AMBIENT: f64 = 0.40;
const SKY: f64 = 0.10;
const SUN: f64 = 0.55;
/// Darkening of the implied ground in full shadow.
const GROUND_SHADOW: f64 = 0.30;
const OUTLINE_RGB: [f64; 3] = [0.16, 0.16, 0.18];
const OUTLINE_MIX: f64 = 0.75;
/// Faces meeting at more than about 20° get an outline.
const OUTLINE_COS: f64 = 0.94;
/// Ambient-occlusion sample pattern: two rings of eight directions, the inner one turned 22.5°.
const AO_OFFSETS: [[f64; 2]; 16] = [
    [1.0, 0.0],
    [FRAC_1_SQRT_2, FRAC_1_SQRT_2],
    [0.0, 1.0],
    [-FRAC_1_SQRT_2, FRAC_1_SQRT_2],
    [-1.0, 0.0],
    [-FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
    [0.0, -1.0],
    [FRAC_1_SQRT_2, -FRAC_1_SQRT_2],
    [0.369_551_813_004_514_9, 0.153_073_372_946_035_8],
    [0.153_073_372_946_035_8, 0.369_551_813_004_514_9],
    [-0.153_073_372_946_035_8, 0.369_551_813_004_514_9],
    [-0.369_551_813_004_514_9, 0.153_073_372_946_035_8],
    [-0.369_551_813_004_514_9, -0.153_073_372_946_035_8],
    [-0.153_073_372_946_035_8, -0.369_551_813_004_514_9],
    [0.153_073_372_946_035_8, -0.369_551_813_004_514_9],
    [0.369_551_813_004_514_9, -0.153_073_372_946_035_8],
];
/// Ambient-occlusion radius as a share of the model's diagonal.
const AO_RADIUS: f64 = 0.05;
const AO_STRENGTH: f64 = 0.8;

/// A count or index as a coordinate; sample counts stay far below 2³².
fn real(value: usize) -> f64 {
    u32::try_from(value).map_or(f64::from(u32::MAX), f64::from)
}
/// A coordinate already clamped to the buffer as its index (towards zero).
fn index(value: f64) -> usize {
    value as usize
}
/// A depth or colour stored in single precision.
fn narrow(value: f64) -> f32 {
    value as f32
}
/// A 0–1 share as an 8-bit channel.
fn channel(value: f64) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

/// An orthonormal pair perpendicular to the unit `axis`.
fn basis(axis: Vec3) -> Option<(Vec3, Vec3)> {
    let reference = if axis.z.abs() > 0.999 {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let first = axis.cross(reference).normalized()?;
    Some((first, first.cross(axis)))
}

fn bounds(points: &[Vec3]) -> Option<(Vec3, Vec3)> {
    let first = *points.first()?;
    Some(points.iter().fold((first, first), |(low, high), point| {
        (
            Vec3::new(low.x.min(point.x), low.y.min(point.y), low.z.min(point.z)),
            Vec3::new(
                high.x.max(point.x),
                high.y.max(point.y),
                high.z.max(point.z),
            ),
        )
    }))
}

struct Camera {
    eye: Vec3,
    forward: Vec3,
    right: Vec3,
    up: Vec3,
    tan_x: f64,
    tan_y: f64,
    width: f64,
    height: f64,
}

impl Camera {
    /// Looks along `-view_from` at the model's box centre, far enough that every corner fits.
    fn fit(corners: &[Vec3], view_from: Vec3, width: usize, height: usize) -> Option<Self> {
        let forward = (-view_from).normalized()?;
        let (right, up) = basis(forward)?;
        let (low, high) = bounds(corners)?;
        let centre = (low + high) * 0.5;
        let diagonal = low.distance(high).max(1e-6);
        let (width, height) = (real(width), real(height));
        let tan_y = TAN_HALF_FOV;
        let tan_x = tan_y * width / height;
        let mut distance = 0.0_f64;
        let mut nearest = f64::NEG_INFINITY;
        for corner in corners {
            let offset = *corner - centre;
            let depth = offset.dot(forward);
            distance = distance
                .max(offset.dot(right).abs() / (tan_x * FRAME_FILL) - depth)
                .max(offset.dot(up).abs() / (tan_y * FRAME_FILL) - depth);
            nearest = nearest.max(-depth);
        }
        let distance = distance.max(nearest + diagonal * 0.01);
        Some(Self {
            eye: centre - forward * distance,
            forward,
            right,
            up,
            tan_x,
            tan_y,
            width,
            height,
        })
    }

    /// Sample-space x, y and view depth of a world point in front of the eye.
    fn project(&self, point: Vec3) -> Option<Vec3> {
        let offset = point - self.eye;
        let depth = offset.dot(self.forward);
        (depth > 1e-9).then(|| {
            Vec3::new(
                (offset.dot(self.right) / (depth * self.tan_x) + 1.0) * 0.5 * self.width,
                (1.0 - offset.dot(self.up) / (depth * self.tan_y)) * 0.5 * self.height,
                depth,
            )
        })
    }

    /// Direction through the centre of sample `at`, scaled so its view depth is 1.
    fn ray(&self, at: usize) -> Vec3 {
        let width = index(self.width);
        let ndc_x = (real(at % width) + 0.5) / self.width * 2.0 - 1.0;
        let ndc_y = 1.0 - (real(at / width) + 0.5) / self.height * 2.0;
        self.forward + self.right * (ndc_x * self.tan_x) + self.up * (ndc_y * self.tan_y)
    }

    fn triangle(&self, corners: [Vec3; 3]) -> Option<[Vec3; 3]> {
        Some([
            self.project(corners[0])?,
            self.project(corners[1])?,
            self.project(corners[2])?,
        ])
    }
}

/// Calls `visit(sample index, depth)` for each sample centre inside the triangle `corners`
/// (x, y, depth). Depth is interpolated perspective-correctly or, for an orthographic map,
/// linearly.
fn raster(
    width: usize,
    height: usize,
    corners: [Vec3; 3],
    perspective: bool,
    mut visit: impl FnMut(usize, f64),
) {
    let flat = |point: Vec3| Vec3::new(point.x, point.y, 0.0);
    let edge = |a: Vec3, b: Vec3, x: f64, y: f64| {
        (flat(b) - flat(a)).cross(Vec3::new(x, y, 0.0) - flat(a)).z
    };
    let [a, b, c] = corners;
    let area = edge(a, b, c.x, c.y);
    if !area.is_finite() || area.abs() < 1e-12 {
        return;
    }
    let min_x = index(a.x.min(b.x).min(c.x).floor().max(0.0));
    let max_x = index(a.x.max(b.x).max(c.x).ceil().clamp(0.0, real(width)));
    let min_y = index(a.y.min(b.y).min(c.y).floor().max(0.0));
    let max_y = index(a.y.max(b.y).max(c.y).ceil().clamp(0.0, real(height)));
    let inverse = [1.0 / a.z, 1.0 / b.z, 1.0 / c.z];
    for y in min_y..max_y {
        let sample_y = real(y) + 0.5;
        for x in min_x..max_x {
            let sample_x = real(x) + 0.5;
            let weights = [
                edge(b, c, sample_x, sample_y) / area,
                edge(c, a, sample_x, sample_y) / area,
                edge(a, b, sample_x, sample_y) / area,
            ];
            if weights.iter().any(|weight| *weight < 0.0) {
                continue;
            }
            let depth = if perspective {
                1.0 / Vec3::from(weights).dot(Vec3::from(inverse))
            } else {
                weights[0] * a.z + weights[1] * b.z + weights[2] * c.z
            };
            visit(y * width + x, depth);
        }
    }
}

/// Depth towards the sun of the opaque parts, seen along the sun's direction.
struct ShadowMap {
    sun: Vec3,
    u_axis: Vec3,
    v_axis: Vec3,
    origin: [f64; 2],
    texel: f64,
    nearest: Vec<f64>,
}

impl ShadowMap {
    fn new(sun: Vec3, triangles: &[[Vec3; 3]]) -> Option<Self> {
        let (u_axis, v_axis) = basis(sun)?;
        let (low, high) = bounds(
            &triangles
                .iter()
                .flatten()
                .map(|point| Vec3::new(point.dot(u_axis), point.dot(v_axis), 0.0))
                .collect::<Vec<_>>(),
        )?;
        let span = (high.x - low.x).max(high.y - low.y).max(1e-6) * 1.04;
        let mut map = Self {
            sun,
            u_axis,
            v_axis,
            origin: [low.x - span * 0.02, low.y - span * 0.02],
            texel: span / real(SHADOW_MAP_SIDE),
            nearest: vec![f64::NEG_INFINITY; SHADOW_MAP_SIDE * SHADOW_MAP_SIDE],
        };
        for triangle in triangles {
            let corners = triangle.map(|point| map.map_point(point));
            let nearest = &mut map.nearest;
            raster(
                SHADOW_MAP_SIDE,
                SHADOW_MAP_SIDE,
                corners,
                false,
                |at, depth| {
                    if depth > nearest[at] {
                        nearest[at] = depth;
                    }
                },
            );
        }
        Some(map)
    }

    fn map_point(&self, point: Vec3) -> Vec3 {
        Vec3::new(
            (point.dot(self.u_axis) - self.origin[0]) / self.texel,
            (point.dot(self.v_axis) - self.origin[1]) / self.texel,
            point.dot(self.sun),
        )
    }

    /// Share of the sun a point with normal `normal` receives, 0–1, softened over 5 × 5 texels.
    fn lit(&self, point: Vec3, normal: Vec3) -> f64 {
        let mapped = self.map_point(point + normal * (self.texel * 1.5));
        let side = real(SHADOW_MAP_SIDE);
        let mut lit = 0;
        for step_v in -2_i32..=2 {
            for step_u in -2_i32..=2 {
                let u = (mapped.x + f64::from(step_u) * 1.25).floor();
                let v = (mapped.y + f64::from(step_v) * 1.25).floor();
                let blocked = u >= 0.0
                    && v >= 0.0
                    && u < side
                    && v < side
                    && self.nearest[index(v) * SHADOW_MAP_SIDE + index(u)] > mapped.z + self.texel;
                lit += i32::from(!blocked);
            }
        }
        f64::from(lit) / 25.0
    }
}

/// The opaque parts as seen from the camera: nearest depth and triangle per sample.
struct Frame<'a> {
    camera: Camera,
    width: usize,
    height: usize,
    normals: &'a [Vec3],
    depth: Vec<f32>,
    triangle: Vec<u32>,
}

impl Frame<'_> {
    fn point(&self, at: usize) -> Vec3 {
        self.camera.eye + self.camera.ray(at) * f64::from(self.depth[at])
    }

    /// Normal of the side of a triangle that faces the eye at a sample.
    fn facing_normal(&self, triangle: usize, at: usize) -> Vec3 {
        let normal = self.normals[triangle];
        if normal.dot(self.camera.ray(at)) > 0.0 {
            -normal
        } else {
            normal
        }
    }

    /// Light kept in corners and creases: 1 in the open, lower where nearby surfaces close in.
    fn ambient_occlusion(&self, at: usize, normal: Vec3, radius: f64) -> f64 {
        let point = self.point(at);
        let height = real(self.height);
        let pixels_per_mm = height * 0.5 / (f64::from(self.depth[at]) * self.camera.tan_y);
        let reach = (radius * pixels_per_mm).clamp(1.5, height * 0.1);
        let (x, y) = (real(at % self.width), real(at / self.width));
        let mut occlusion = 0.0;
        for [dx, dy] in AO_OFFSETS {
            let (sx, sy) = ((x + dx * reach).round(), (y + dy * reach).round());
            if sx < 0.0 || sy < 0.0 || sx >= real(self.width) || sy >= height {
                continue;
            }
            let other = index(sy) * self.width + index(sx);
            if self.triangle[other] == NO_TRIANGLE {
                continue;
            }
            let towards = self.point(other) - point;
            let distance = towards.length();
            if distance < 1e-9 || distance > radius {
                continue;
            }
            let share = distance / radius;
            occlusion += (normal.dot(towards) / distance - 0.15).max(0.0) * (1.0 - share * share);
        }
        1.0 - AO_STRENGTH * (occlusion * 3.0 / real(AO_OFFSETS.len())).min(1.0)
    }

    /// An outline sample: it meets another surface at a crease, a step or the background.
    fn outline(&self, at: usize) -> bool {
        let own = self.triangle[at];
        if own == NO_TRIANGLE {
            return false;
        }
        let (x, y) = (at % self.width, at / self.width);
        let neighbours = [
            (x + 1 < self.width).then_some(at + 1),
            (x > 0).then_some(at.wrapping_sub(1)),
            (y + 1 < self.height).then_some(at + self.width),
            (y > 0).then_some(at.wrapping_sub(self.width)),
        ];
        neighbours.into_iter().flatten().any(|other| {
            let theirs = self.triangle[other];
            if theirs == own {
                return false;
            }
            if theirs == NO_TRIANGLE {
                return true;
            }
            let (Ok(own), Ok(theirs)) = (usize::try_from(own), usize::try_from(theirs)) else {
                return false;
            };
            let crease = self.normals[own].dot(self.normals[theirs]).abs() < OUTLINE_COS;
            let step = self.depth[other] > self.depth[at] * 1.02;
            crease || step
        })
    }
}

fn shade(base: [u8; 4], normal: Vec3, sun: Vec3, lit: f64, occlusion: f64) -> [f64; 3] {
    let light =
        (AMBIENT + SKY * (normal.z * 0.5 + 0.5)) * occlusion + SUN * normal.dot(sun).max(0.0) * lit;
    [0, 1, 2].map(|c| (f64::from(base[c]) / 255.0 * light).min(1.0))
}

/// The triangles that have an area and are not fully transparent, with their unit normals.
struct Surfaces {
    corners: Vec<[Vec3; 3]>,
    normals: Vec<Vec3>,
    colours: Vec<[u8; 4]>,
}

impl Surfaces {
    fn new(scene: &PresentationScene) -> Self {
        let mut surfaces = Self {
            corners: Vec::with_capacity(scene.triangles.len()),
            normals: Vec::with_capacity(scene.triangles.len()),
            colours: Vec::with_capacity(scene.triangles.len()),
        };
        for triangle in &scene.triangles {
            let corners = triangle.corners_mm.map(Vec3::from);
            let [a, b, c] = corners;
            if triangle.rgba[3] == 0 {
                continue;
            }
            if let Some(normal) = (b - a).cross(c - a).normalized() {
                surfaces.corners.push(corners);
                surfaces.normals.push(normal);
                surfaces.colours.push(triangle.rgba);
            }
        }
        surfaces
    }

    fn with_opacity(&self, opaque: bool) -> impl Iterator<Item = usize> + '_ {
        (0..self.colours.len()).filter(move |at| (self.colours[*at][3] == 255) == opaque)
    }
}

pub(super) fn render(
    scene: &PresentationScene,
    options: &PresentationOptions,
) -> Result<PresentationImage, PresentationError> {
    let size_error = |_out_of_range: std::num::TryFromIntError| PresentationError::Size {
        width: options.width,
        height: options.height,
        max_side: super::MAX_PRESENTATION_SIDE,
    };
    let factor = usize::try_from(options.supersample).map_err(size_error)?;
    let width = usize::try_from(options.width).map_err(size_error)? * factor;
    let height = usize::try_from(options.height).map_err(size_error)? * factor;
    let surfaces = Surfaces::new(scene);
    let corners = surfaces
        .corners
        .iter()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let (low, high) = bounds(&corners).ok_or(PresentationError::EmptyModel)?;
    let view_from = Vec3::from(options.view_from);
    if !view_from.is_finite() {
        return Err(PresentationError::ViewDirection);
    }
    let camera =
        Camera::fit(&corners, view_from, width, height).ok_or(PresentationError::ViewDirection)?;
    // The sun shines from above, behind the viewer's left shoulder.
    let sun = (Vec3::Z - camera.right * 0.5 - camera.forward * 0.4)
        .normalized()
        .ok_or(PresentationError::ViewDirection)?;
    let opaque = surfaces.with_opacity(true).collect::<Vec<_>>();
    let shadow_map = if options.shadows {
        ShadowMap::new(
            sun,
            &opaque
                .iter()
                .map(|at| surfaces.corners[*at])
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut frame = Frame {
        camera,
        width,
        height,
        normals: &surfaces.normals,
        depth: vec![f32::INFINITY; width * height],
        triangle: vec![NO_TRIANGLE; width * height],
    };
    for &at in &opaque {
        let Some(projected) = frame.camera.triangle(surfaces.corners[at]) else {
            continue;
        };
        let id = u32::try_from(at).map_err(|_out_of_range: std::num::TryFromIntError| {
            PresentationError::TooManyTriangles {
                triangles: surfaces.corners.len(),
                limit: super::limits::EXPORT_TRIANGLES,
            }
        })?;
        let (depth, owner) = (&mut frame.depth, &mut frame.triangle);
        raster(width, height, projected, true, |sample, value| {
            let value = narrow(value);
            if value < depth[sample] {
                depth[sample] = value;
                owner[sample] = id;
            }
        });
    }
    let shading = Shading {
        frame: &frame,
        surfaces: &surfaces,
        shadow_map: shadow_map.as_ref(),
        sun,
        ground: low.z,
        ao_radius: low.distance(high) * AO_RADIUS,
        options,
    };
    let mut colour = shading.opaque();
    shading.see_through(&mut colour);
    Ok(downsample(options, factor, width, &colour))
}

struct Shading<'a> {
    frame: &'a Frame<'a>,
    surfaces: &'a Surfaces,
    shadow_map: Option<&'a ShadowMap>,
    sun: Vec3,
    ground: f64,
    ao_radius: f64,
    options: &'a PresentationOptions,
}

impl Shading<'_> {
    /// Premultiplied RGBA of each sample: the opaque parts over the background and the shadow on
    /// the implied ground at the model's lowest point.
    fn opaque(&self) -> Vec<[f32; 4]> {
        let background = match self.options.background {
            Background::White => [1.0, 1.0, 1.0, 1.0],
            Background::Transparent => [0.0, 0.0, 0.0, 0.0],
            Background::Color([r, g, b]) => [
                f64::from(r) / 255.0,
                f64::from(g) / 255.0,
                f64::from(b) / 255.0,
                1.0,
            ],
        };
        let frame = self.frame;
        (0..frame.depth.len())
            .map(|at| {
                let owner = frame.triangle[at];
                let part = (owner != NO_TRIANGLE)
                    .then(|| usize::try_from(owner).ok())
                    .flatten();
                let pixel = match part {
                    Some(owner) => self.surface(owner, at),
                    None => {
                        let shadow = self.ground_shadow(at);
                        if background[3] == 0.0 {
                            [0.0, 0.0, 0.0, shadow]
                        } else {
                            let keep = 1.0 - shadow;
                            [
                                background[0] * keep,
                                background[1] * keep,
                                background[2] * keep,
                                1.0,
                            ]
                        }
                    }
                };
                pixel.map(narrow)
            })
            .collect()
    }

    fn ground_shadow(&self, at: usize) -> f64 {
        let camera = &self.frame.camera;
        let ray = camera.ray(at);
        match self.shadow_map {
            Some(map) if ray.z < 0.0 && camera.eye.z > self.ground => {
                let point = camera.eye + ray * ((self.ground - camera.eye.z) / ray.z);
                GROUND_SHADOW * (1.0 - map.lit(point, Vec3::Z))
            }
            _ => 0.0,
        }
    }

    fn surface(&self, owner: usize, at: usize) -> [f64; 4] {
        let frame = self.frame;
        let normal = frame.facing_normal(owner, at);
        let lit = self
            .shadow_map
            .map_or(1.0, |map| map.lit(frame.point(at), normal));
        let occlusion = if self.options.ambient_occlusion {
            frame.ambient_occlusion(at, normal, self.ao_radius)
        } else {
            1.0
        };
        let mut rgb = shade(
            self.surfaces.colours[owner],
            normal,
            self.sun,
            lit,
            occlusion,
        );
        if self.options.outlines && frame.outline(at) {
            rgb = [0, 1, 2].map(|c| rgb[c] * (1.0 - OUTLINE_MIX) + OUTLINE_RGB[c] * OUTLINE_MIX);
        }
        [rgb[0], rgb[1], rgb[2], 1.0]
    }

    /// Blends the see-through parts, farthest first, over the samples they are in front of.
    fn see_through(&self, colour: &mut [[f32; 4]]) {
        let frame = self.frame;
        let camera = &frame.camera;
        let mut layers = self
            .surfaces
            .with_opacity(false)
            .filter_map(|at| {
                let [a, b, c] = self.surfaces.corners[at];
                let depth = ((a + b + c) * (1.0 / 3.0) - camera.eye).dot(camera.forward);
                Some((depth, at, camera.triangle(self.surfaces.corners[at])?))
            })
            .collect::<Vec<_>>();
        layers.sort_by(|left, right| right.0.total_cmp(&left.0).then(left.1.cmp(&right.1)));
        for (_, at, projected) in layers {
            let alpha = f64::from(self.surfaces.colours[at][3]) / 255.0;
            raster(
                frame.width,
                frame.height,
                projected,
                true,
                |sample, depth| {
                    if narrow(depth) >= frame.depth[sample] {
                        return;
                    }
                    let normal = frame.facing_normal(at, sample);
                    let point = camera.eye + camera.ray(sample) * depth;
                    let lit = self.shadow_map.map_or(1.0, |map| map.lit(point, normal));
                    let rgb = shade(self.surfaces.colours[at], normal, self.sun, lit, 1.0);
                    let front = [rgb[0], rgb[1], rgb[2], 1.0];
                    let below = colour[sample];
                    colour[sample] = [0, 1, 2, 3]
                        .map(|c| narrow(front[c] * alpha + f64::from(below[c]) * (1.0 - alpha)));
                },
            );
        }
    }
}

fn downsample(
    options: &PresentationOptions,
    factor: usize,
    sample_width: usize,
    colour: &[[f32; 4]],
) -> PresentationImage {
    let (width, height) = (sample_width / factor, colour.len() / sample_width / factor);
    let count = real(factor * factor);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0.0_f64; 4];
            for row in 0..factor {
                let start = (y * factor + row) * sample_width + x * factor;
                for sample in &colour[start..start + factor] {
                    for (total, value) in sum.iter_mut().zip(sample) {
                        *total += f64::from(*value);
                    }
                }
            }
            let straight = |value: f64| if sum[3] > 0.0 { value / sum[3] } else { 0.0 };
            for value in [
                straight(sum[0]),
                straight(sum[1]),
                straight(sum[2]),
                sum[3] / count,
            ] {
                rgba.push(channel(value));
            }
        }
    }
    PresentationImage {
        width: options.width,
        height: options.height,
        rgba,
    }
}
