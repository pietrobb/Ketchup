//! Census of Fillet and Shell on ordinary bodies: which cases are made and
//! which are refused today. Every case has an expected outcome, so a change
//! that turns a refusal into a result, or a result into a refusal, shows up
//! here as one line of the table.

use ketchup_exact::{
    BoxSpec, CircleExtrudeSpec, EdgeFinish, ExactBackend, ExactBody, ExactOpOutput, GeometryError,
    PlanarProfileLoop, PlanarProfileSegment, Point3, ShellDirection, Size3,
};

const LENGTH: f64 = 40.0;
const WIDTH: f64 = 30.0;
const HEIGHT: f64 = 20.0;

fn block(backend: &ExactBackend) -> ExactOpOutput {
    backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: LENGTH,
                y: WIDTH,
                z: HEIGHT,
            },
        })
        .expect("block")
}

fn cylinder(backend: &ExactBackend) -> ExactOpOutput {
    backend
        .extrude_circle(CircleExtrudeSpec {
            center_mm: [0.0, 0.0],
            radius_mm: 10.0,
            height_mm: HEIGHT,
        })
        .expect("cylinder")
}

fn line(start_mm: [f64; 2], end_mm: [f64; 2]) -> PlanarProfileSegment {
    PlanarProfileSegment::Line { start_mm, end_mm }
}

fn polygon(points: &[[f64; 2]]) -> PlanarProfileLoop {
    PlanarProfileLoop::Segments(
        (0..points.len())
            .map(|index| line(points[index], points[(index + 1) % points.len()]))
            .collect(),
    )
}

fn segments(planar_loop: PlanarProfileLoop) -> Vec<PlanarProfileSegment> {
    match planar_loop {
        PlanarProfileLoop::Segments(segments) => segments,
        PlanarProfileLoop::Circle { .. } => unreachable!("polygons are segments"),
    }
}

/// An L whose inside vertical edge at (10, 10) is concave.
fn ell(backend: &ExactBackend) -> ExactOpOutput {
    backend
        .extrude_mixed_profile(
            &segments(polygon(&[
                [0.0, 0.0],
                [40.0, 0.0],
                [40.0, 10.0],
                [10.0, 10.0],
                [10.0, 30.0],
                [0.0, 30.0],
            ])),
            HEIGHT,
        )
        .expect("ell")
}

/// A 40 × 30 plate, 10 thick, with a 12 mm through hole in the middle.
fn holed_plate(backend: &ExactBackend) -> ExactOpOutput {
    backend
        .extrude_planar_region(
            &polygon(&[[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]),
            &[PlanarProfileLoop::Circle {
                center_mm: [20.0, 15.0],
                radius_mm: 6.0,
            }],
            10.0,
        )
        .expect("holed plate")
}

/// A 40 × 30 rectangle with 5 mm corner arcs: its top edges form one tangent chain.
fn rounded_block(backend: &ExactBackend) -> ExactOpOutput {
    let r = 5.0;
    let arc = |start_mm, end_mm, center_mm| PlanarProfileSegment::CircularArc {
        start_mm,
        end_mm,
        center_mm,
        clockwise: false,
    };
    backend
        .extrude_mixed_profile(
            &[
                line([r, 0.0], [LENGTH - r, 0.0]),
                arc([LENGTH - r, 0.0], [LENGTH, r], [LENGTH - r, r]),
                line([LENGTH, r], [LENGTH, WIDTH - r]),
                arc(
                    [LENGTH, WIDTH - r],
                    [LENGTH - r, WIDTH],
                    [LENGTH - r, WIDTH - r],
                ),
                line([LENGTH - r, WIDTH], [r, WIDTH]),
                arc([r, WIDTH], [0.0, WIDTH - r], [r, WIDTH - r]),
                line([0.0, WIDTH - r], [0.0, r]),
                arc([0.0, r], [r, 0.0], [r, r]),
            ],
            HEIGHT,
        )
        .expect("rounded block")
}

fn near(point: &Point3, target: [f64; 3]) -> bool {
    (point.x - target[0]).abs() < 1.0e-6
        && (point.y - target[1]).abs() < 1.0e-6
        && (point.z - target[2]).abs() < 1.0e-6
}

/// Ordinals of the edges whose centroids are the given points, sorted.
fn edges_at(body: &ExactBody, centroids: &[[f64; 3]]) -> Vec<u32> {
    let mut ordinals = centroids
        .iter()
        .map(|target| {
            body.topology
                .edges
                .iter()
                .find(|edge| near(&edge.centroid_mm, *target))
                .unwrap_or_else(|| panic!("no edge centred at {target:?}"))
                .ordinal
        })
        .collect::<Vec<_>>();
    ordinals.sort_unstable();
    ordinals
}

fn all_edges(body: &ExactBody) -> Vec<u32> {
    (0..body.topology.edge_count).collect()
}

/// Edges lying entirely at height `z`.
fn edges_at_height(body: &ExactBody, z: f64) -> Vec<u32> {
    body.topology
        .edges
        .iter()
        .filter(|edge| {
            (edge.bounds_mm.min.z - z).abs() < 1.0e-6 && (edge.bounds_mm.max.z - z).abs() < 1.0e-6
        })
        .map(|edge| edge.ordinal)
        .collect()
}

/// The planar face whose outward normal is +Z at height `z`.
fn top_face(body: &ExactBody, z: f64) -> u32 {
    body.topology
        .faces
        .iter()
        .find(|face| {
            face.surface_kind == "plane"
                && (face.normal.z - 1.0).abs() < 1.0e-9
                && (face.centroid_mm.z - z).abs() < 1.0e-6
        })
        .expect("top face")
        .ordinal
}

fn bottom_face(body: &ExactBody) -> u32 {
    body.topology
        .faces
        .iter()
        .find(|face| {
            face.surface_kind == "plane"
                && (face.normal.z + 1.0).abs() < 1.0e-9
                && face.centroid_mm.z.abs() < 1.0e-6
        })
        .expect("bottom face")
        .ordinal
}

/// The three edges of the block meeting at its corner (LENGTH, WIDTH, HEIGHT).
fn corner_edges(body: &ExactBody) -> Vec<u32> {
    edges_at(
        body,
        &[
            [LENGTH / 2.0, WIDTH, HEIGHT],
            [LENGTH, WIDTH / 2.0, HEIGHT],
            [LENGTH, WIDTH, HEIGHT / 2.0],
        ],
    )
}

fn vertical_block_edges(body: &ExactBody) -> Vec<u32> {
    edges_at(
        body,
        &[
            [0.0, 0.0, HEIGHT / 2.0],
            [LENGTH, 0.0, HEIGHT / 2.0],
            [LENGTH, WIDTH, HEIGHT / 2.0],
            [0.0, WIDTH, HEIGHT / 2.0],
        ],
    )
}

fn fillet(backend: &ExactBackend, body: &ExactBody, edges: &[u32], radius: f64) -> Outcome {
    backend
        .finish_body(body, edges, EdgeFinish::Fillet, radius)
        .into()
}

fn variable_fillet(
    backend: &ExactBackend,
    body: &ExactBody,
    edges: &[u32],
    start: f64,
    stations: &[[f64; 2]],
) -> Outcome {
    backend
        .finish_body_with_radius_stations(body, edges, EdgeFinish::Fillet, start, stations)
        .into()
}

fn shell(
    backend: &ExactBackend,
    body: &ExactBody,
    open: &[u32],
    thickness: f64,
    direction: ShellDirection,
) -> Outcome {
    let mut open = open.to_vec();
    open.sort_unstable();
    backend
        .shell_body_with_direction(body, &open, thickness, direction)
        .into()
}

#[derive(Debug)]
enum Outcome {
    Made { volume_mm3: f64 },
    Refused(String),
}

impl From<Result<ExactOpOutput, GeometryError>> for Outcome {
    fn from(result: Result<ExactOpOutput, GeometryError>) -> Self {
        match result {
            Ok(output) => Self::Made {
                volume_mm3: output.body.topology.volume_mm3,
            },
            Err(error) => Self::Refused(error.to_string()),
        }
    }
}

/// What a case must do.
enum Expect {
    /// Made, with this volume worked out by hand (sharp-cornered walls).
    Volume(f64),
    /// Made, with a volume strictly between these bounds.
    Between(f64, f64),
    /// Refused, with an explanation containing this text.
    Refused(&'static str),
}

impl Expect {
    fn met_by(&self, outcome: &Outcome) -> bool {
        match (self, outcome) {
            (Self::Volume(expected), Outcome::Made { volume_mm3 }) => {
                (volume_mm3 - expected).abs() <= expected * 1.0e-9
            }
            (Self::Between(low, high), Outcome::Made { volume_mm3 }) => {
                low < volume_mm3 && volume_mm3 < high
            }
            (Self::Refused(reason), Outcome::Refused(actual)) => actual.contains(reason),
            _ => false,
        }
    }
}

fn filleted(backend: &ExactBackend, body: &ExactBody, edges: &[u32], radius: f64) -> ExactOpOutput {
    backend
        .finish_body(body, edges, EdgeFinish::Fillet, radius)
        .expect("filleted base")
}

const NOT_COMPLETED: &str = "did not complete";
const NOT_OFFSET: &str = "Shell could not offset the body by the wall thickness";
const STEPPED_CLOSED_RUN: &str = "closed run of edges must end with the radius it starts with";
/// What one 2 mm round takes off a straight edge, per mm of its length.
const ROUND_2_PER_MM: f64 = (1.0 - std::f64::consts::FRAC_PI_4) * 4.0;

const BLOCK_VOLUME: f64 = LENGTH * WIDTH * HEIGHT;
const CYLINDER_VOLUME: f64 = std::f64::consts::PI * 100.0 * HEIGHT;
const ELL_VOLUME: f64 = 600.0 * HEIGHT;
const PLATE_VOLUME: f64 = (1200.0 - std::f64::consts::PI * 36.0) * 10.0;
const ROUNDED_VOLUME: f64 = (1200.0 - (4.0 - std::f64::consts::PI) * 25.0) * HEIGHT;

/// An `x` × `y` × `z` box with every edge rounded to `r`: its flat-faced core,
/// the slabs on its faces, the quarter cylinders on its edges and the sphere
/// its eight corners make up.
fn rounded_box_volume(x: f64, y: f64, z: f64, r: f64) -> f64 {
    let (a, b, c) = (x - 2.0 * r, y - 2.0 * r, z - 2.0 * r);
    a * b * c
        + 2.0 * r * (a * b + b * c + a * c)
        + std::f64::consts::PI * r * r * (a + b + c)
        + 4.0 / 3.0 * std::f64::consts::PI * r.powi(3)
}

/// Made, taking off at most a tenth of `volume`.
fn below(volume: f64) -> Expect {
    Expect::Between(volume * 0.9, volume)
}

fn assert_census(cases: &[(&str, Outcome, Expect)]) {
    let mut mismatches = 0;
    let table = cases
        .iter()
        .map(|(name, outcome, expect)| {
            let mark = if expect.met_by(outcome) {
                "  "
            } else {
                mismatches += 1;
                "!!"
            };
            match outcome {
                Outcome::Made { volume_mm3 } => {
                    format!("{mark} MADE    {name}: {volume_mm3:.3} mm3")
                }
                Outcome::Refused(reason) => format!("{mark} REFUSED {name}: {reason}"),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(mismatches, 0, "cases marked !! differ:\n{table}");
}

fn hole_rim(plate: &ExactBody) -> Vec<u32> {
    plate
        .topology
        .edges
        .iter()
        .filter(|edge| edge.circle_radius_mm == Some(6.0) && edge.bounds_mm.min.z > 5.0)
        .map(|edge| edge.ordinal)
        .collect()
}

#[test]
fn census_of_constant_fillets() {
    let backend = ExactBackend::new();
    let block = block(&backend);
    let block_body = &block.body;
    let cylinder = cylinder(&backend);
    let ell = ell(&backend);
    let plate = holed_plate(&backend);
    let rounded = rounded_block(&backend);
    let top_edge = edges_at(block_body, &[[LENGTH / 2.0, 0.0, HEIGHT]]);
    assert_census(&[
        (
            "fillet one edge",
            fillet(&backend, block_body, &top_edge, 2.0),
            Expect::Volume(BLOCK_VOLUME - ROUND_2_PER_MM * LENGTH),
        ),
        (
            "fillet three edges at a corner",
            fillet(&backend, block_body, &corner_edges(block_body), 2.0),
            below(BLOCK_VOLUME),
        ),
        (
            "fillet every edge",
            fillet(&backend, block_body, &all_edges(block_body), 2.0),
            below(BLOCK_VOLUME),
        ),
        // Rounds meeting across a whole face leave no flat between them.
        (
            "fillet every edge, radius half the height",
            fillet(&backend, block_body, &all_edges(block_body), HEIGHT / 2.0),
            Expect::Refused(NOT_COMPLETED),
        ),
        (
            "fillet one edge, radius above the height",
            fillet(&backend, block_body, &top_edge, HEIGHT + 5.0),
            Expect::Refused(NOT_COMPLETED),
        ),
        (
            "fillet cylinder top circle",
            fillet(
                &backend,
                &cylinder.body,
                &edges_at(&cylinder.body, &[[0.0, 0.0, HEIGHT]]),
                2.0,
            ),
            below(CYLINDER_VOLUME),
        ),
        // A concave round adds material.
        (
            "fillet concave edge",
            fillet(
                &backend,
                &ell.body,
                &edges_at(&ell.body, &[[10.0, 10.0, HEIGHT / 2.0]]),
                2.0,
            ),
            Expect::Volume(ELL_VOLUME + ROUND_2_PER_MM * HEIGHT),
        ),
        (
            "fillet every top edge of an L",
            fillet(
                &backend,
                &ell.body,
                &edges_at_height(&ell.body, HEIGHT),
                2.0,
            ),
            Expect::Between(ELL_VOLUME * 0.9, ELL_VOLUME * 1.01),
        ),
        (
            "fillet hole rim",
            fillet(&backend, &plate.body, &hole_rim(&plate.body), 1.0),
            below(PLATE_VOLUME),
        ),
        (
            "fillet tangent chain",
            fillet(
                &backend,
                &rounded.body,
                &edges_at(&rounded.body, &[[LENGTH / 2.0, 0.0, HEIGHT]]),
                2.0,
            ),
            below(ROUNDED_VOLUME),
        ),
    ]);
}

#[test]
fn census_of_variable_fillets() {
    let backend = ExactBackend::new();
    let block = block(&backend);
    let block_body = &block.body;
    let cylinder = cylinder(&backend);
    let cylinder_top = edges_at(&cylinder.body, &[[0.0, 0.0, HEIGHT]]);
    let rounded = rounded_block(&backend);
    let rounded_front = edges_at(&rounded.body, &[[LENGTH / 2.0, 0.0, HEIGHT]]);
    let top_edge = edges_at(block_body, &[[LENGTH / 2.0, 0.0, HEIGHT]]);
    assert_census(&[
        (
            "variable fillet one edge",
            variable_fillet(&backend, block_body, &top_edge, 1.0, &[[1.0, 3.0]]),
            below(BLOCK_VOLUME),
        ),
        (
            "variable fillet three edges at a corner",
            variable_fillet(
                &backend,
                block_body,
                &corner_edges(block_body),
                1.0,
                &[[1.0, 3.0]],
            ),
            below(BLOCK_VOLUME),
        ),
        (
            "variable fillet four vertical edges",
            variable_fillet(
                &backend,
                block_body,
                &vertical_block_edges(block_body),
                1.0,
                &[[0.5, 3.0], [1.0, 1.0]],
            ),
            below(BLOCK_VOLUME),
        ),
        // Edges meeting at corners are separate runs, each with its own ends.
        (
            "variable fillet every top edge",
            variable_fillet(
                &backend,
                block_body,
                &edges_at_height(block_body, HEIGHT),
                1.0,
                &[[1.0, 3.0]],
            ),
            below(BLOCK_VOLUME),
        ),
        (
            "variable fillet cylinder top circle, equal ends",
            variable_fillet(
                &backend,
                &cylinder.body,
                &cylinder_top,
                1.0,
                &[[0.5, 3.0], [1.0, 1.0]],
            ),
            below(CYLINDER_VOLUME),
        ),
        // OCCT made these with a step in the round where the run closes.
        (
            "variable fillet cylinder top circle, unequal ends",
            variable_fillet(&backend, &cylinder.body, &cylinder_top, 1.0, &[[1.0, 3.0]]),
            Expect::Refused(STEPPED_CLOSED_RUN),
        ),
        (
            "variable fillet tangent chain",
            variable_fillet(&backend, &rounded.body, &rounded_front, 1.0, &[[1.0, 3.0]]),
            Expect::Refused(STEPPED_CLOSED_RUN),
        ),
    ]);
}

#[test]
fn census_of_shells_of_sharp_bodies() {
    let backend = ExactBackend::new();
    let block = block(&backend);
    let block_body = &block.body;
    let block_top = top_face(block_body, HEIGHT);
    let cylinder = cylinder(&backend);
    let ell = ell(&backend);
    let plate = holed_plate(&backend);
    let rounded = rounded_block(&backend);
    assert_census(&[
        (
            "shell block open top inward",
            shell(
                &backend,
                block_body,
                &[block_top],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Volume(BLOCK_VOLUME - 36.0 * 26.0 * 18.0),
        ),
        (
            "shell block open top outward",
            shell(
                &backend,
                block_body,
                &[block_top],
                2.0,
                ShellDirection::Outward,
            ),
            Expect::Volume(44.0 * 34.0 * 22.0 - BLOCK_VOLUME),
        ),
        (
            "shell block open top symmetric",
            shell(
                &backend,
                block_body,
                &[block_top],
                2.0,
                ShellDirection::Symmetric,
            ),
            Expect::Volume(42.0 * 32.0 * 21.0 - 38.0 * 28.0 * 19.0),
        ),
        (
            "shell block open top and bottom",
            shell(
                &backend,
                block_body,
                &[block_top, bottom_face(block_body)],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Volume(BLOCK_VOLUME - 36.0 * 26.0 * HEIGHT),
        ),
        // OCCT handed the solid block back as if it were shelled; its offset
        // by the wall does not exist.
        (
            "shell block thicker than half its width",
            shell(
                &backend,
                block_body,
                &[block_top],
                16.0,
                ShellDirection::Inward,
            ),
            Expect::Refused(NOT_OFFSET),
        ),
        (
            "shell cylinder open top",
            shell(
                &backend,
                &cylinder.body,
                &[top_face(&cylinder.body, HEIGHT)],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Volume(CYLINDER_VOLUME - std::f64::consts::PI * 64.0 * 18.0),
        ),
        (
            "shell L open top",
            shell(
                &backend,
                &ell.body,
                &[top_face(&ell.body, HEIGHT)],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Volume(ELL_VOLUME - (36.0 * 6.0 + 6.0 * 20.0) * 18.0),
        ),
        (
            "shell holed plate open top",
            shell(
                &backend,
                &plate.body,
                &[top_face(&plate.body, 10.0)],
                1.0,
                ShellDirection::Inward,
            ),
            Expect::Volume(PLATE_VOLUME - (38.0 * 28.0 - std::f64::consts::PI * 49.0) * 9.0),
        ),
        (
            "shell rounded profile open top",
            shell(
                &backend,
                &rounded.body,
                &[top_face(&rounded.body, HEIGHT)],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Volume(
                ROUNDED_VOLUME - (36.0 * 26.0 - (4.0 - std::f64::consts::PI) * 9.0) * 18.0,
            ),
        ),
    ]);
}

#[test]
fn census_of_shells_of_filleted_bodies() {
    let backend = ExactBackend::new();
    let block = block(&backend);
    let block_body = &block.body;
    let rounded_all = filleted(&backend, block_body, &all_edges(block_body), 3.0);
    let thin_rounded = filleted(&backend, block_body, &all_edges(block_body), 1.0);
    let thin_top = top_face(&thin_rounded.body, HEIGHT);
    let variable_rounded = backend
        .finish_body_with_radius_stations(
            block_body,
            &vertical_block_edges(block_body),
            EdgeFinish::Fillet,
            2.0,
            &[[0.5, 4.0], [1.0, 2.0]],
        )
        .expect("variable rounded base");
    let variable_rounded_volume = variable_rounded.body.topology.volume_mm3;
    let small_vertical = filleted(&backend, block_body, &vertical_block_edges(block_body), 1.0);
    assert_census(&[
        // The top blends into its rounds, which OCCT's thick solid cannot open.
        // The rounds are larger than the wall, so the inside is the block
        // inset by the wall with 1 mm rounds; the opening is the flat of the top.
        (
            "shell filleted block open top, fillet above wall",
            shell(
                &backend,
                &rounded_all.body,
                &[top_face(&rounded_all.body, HEIGHT)],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Volume(
                rounded_box_volume(LENGTH, WIDTH, HEIGHT, 3.0)
                    - rounded_box_volume(36.0, 26.0, 16.0, 1.0)
                    - 34.0 * 24.0 * 2.0,
            ),
        ),
        // The outside is the block grown by the wall with 3 mm rounds.
        (
            "shell filleted block outward, fillet below wall",
            shell(
                &backend,
                &thin_rounded.body,
                &[thin_top],
                2.0,
                ShellDirection::Outward,
            ),
            Expect::Volume(
                rounded_box_volume(44.0, 34.0, 24.0, 3.0)
                    - rounded_box_volume(LENGTH, WIDTH, HEIGHT, 1.0)
                    - 38.0 * 28.0 * 2.0,
            ),
        ),
        // Rounds smaller than the wall would vanish inside it; OCCT turns them
        // inside out instead, so the offset is refused.
        (
            "shell filleted block open top, fillet below wall",
            shell(
                &backend,
                &thin_rounded.body,
                &[thin_top],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Refused(NOT_OFFSET),
        ),
        (
            "shell filleted block closed, fillet below wall",
            shell(
                &backend,
                &thin_rounded.body,
                &[],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Refused(NOT_COMPLETED),
        ),
        // The rounds stop at the open top; the wall still swallows them.
        (
            "shell block with small vertical rounds open top",
            shell(
                &backend,
                &small_vertical.body,
                &[top_face(&small_vertical.body, HEIGHT)],
                2.0,
                ShellDirection::Inward,
            ),
            Expect::Refused(NOT_OFFSET),
        ),
        (
            "shell variable-filleted block open top",
            shell(
                &backend,
                &variable_rounded.body,
                &[top_face(&variable_rounded.body, HEIGHT)],
                1.0,
                ShellDirection::Inward,
            ),
            Expect::Between(variable_rounded_volume * 0.1, variable_rounded_volume * 0.2),
        ),
    ]);
}
