use super::blender_glb_export::{current_package, seeded_document};
use ketchup_manufacturing::blender_export::MeshGlbInstance;
use ketchup_manufacturing::presentation::{
    Background, PresentationError, PresentationImage, PresentationOptions, PresentationScene,
    PresentationTriangle, presentation_scene, render_presentation,
};
use ketchup_model::exact_product::MeshExportSource;

const REFERENCE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/presentation/box_on_slab.png"
);

fn cuboid(low: [f64; 3], high: [f64; 3], rgba: [u8; 4]) -> Vec<PresentationTriangle> {
    let corner = |i: usize| {
        [
            if i & 1 == 0 { low[0] } else { high[0] },
            if i & 2 == 0 { low[1] } else { high[1] },
            if i & 4 == 0 { low[2] } else { high[2] },
        ]
    };
    let faces = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    faces
        .iter()
        .flat_map(|[a, b, c, d]| {
            [
                PresentationTriangle {
                    corners_mm: [corner(*a), corner(*b), corner(*c)],
                    rgba,
                },
                PresentationTriangle {
                    corners_mm: [corner(*a), corner(*c), corner(*d)],
                    rgba,
                },
            ]
        })
        .collect()
}

/// A grey slab with a red box standing on it.
fn box_on_slab() -> PresentationScene {
    let mut triangles = cuboid(
        [-200.0, -150.0, 0.0],
        [200.0, 150.0, 20.0],
        [190, 190, 185, 255],
    );
    triangles.extend(cuboid(
        [-50.0, -50.0, 20.0],
        [50.0, 50.0, 140.0],
        [200, 60, 50, 255],
    ));
    PresentationScene { triangles }
}

fn small(background: Background) -> PresentationOptions {
    PresentationOptions {
        width: 160,
        height: 120,
        background,
        supersample: 2,
        ..PresentationOptions::default()
    }
}

fn pixel(image: &PresentationImage, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.width + x) * 4) as usize;
    [
        image.rgba[at],
        image.rgba[at + 1],
        image.rgba[at + 2],
        image.rgba[at + 3],
    ]
}

fn pixels(image: &PresentationImage) -> impl Iterator<Item = [u8; 4]> + '_ {
    image.rgba.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]])
}

fn brightness(image: &PresentationImage) -> u64 {
    pixels(image)
        .map(|p| u64::from(p[0]) + u64::from(p[1]) + u64::from(p[2]))
        .sum()
}

fn decode(png_bytes: &[u8]) -> PresentationImage {
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut rgba = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut rgba).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    rgba.truncate(info.buffer_size());
    PresentationImage {
        width: info.width,
        height: info.height,
        rgba,
    }
}

#[test]
fn the_image_matches_its_reference_within_tolerance_and_renders_identically_twice() {
    let options = small(Background::White);
    let image = render_presentation(&box_on_slab(), &options).unwrap();
    assert_eq!(
        image,
        render_presentation(&box_on_slab(), &options).unwrap()
    );
    let png_bytes = image.png().unwrap();
    assert_eq!(decode(&png_bytes), image);
    // A changed renderer is reviewed with this image beside the reference before replacing it.
    std::fs::write(
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("presentation_actual.png"),
        &png_bytes,
    )
    .unwrap();
    let reference = decode(&std::fs::read(REFERENCE).unwrap());
    assert_eq!((reference.width, reference.height), (160, 120));
    let differences = image
        .rgba
        .iter()
        .zip(&reference.rgba)
        .map(|(a, b)| a.abs_diff(*b))
        .collect::<Vec<_>>();
    let mean = differences.iter().map(|d| f64::from(*d)).sum::<f64>() / differences.len() as f64;
    let far = differences.iter().filter(|d| **d > 8).count();
    assert!(mean < 0.5, "mean channel difference {mean}");
    assert!(
        far * 200 < differences.len(),
        "{far} channels differ by more than 8"
    );
}

#[test]
fn the_background_is_white_a_colour_or_transparent() {
    let white = render_presentation(&box_on_slab(), &small(Background::White)).unwrap();
    assert_eq!(pixel(&white, 0, 0), [255, 255, 255, 255]);
    let blue =
        render_presentation(&box_on_slab(), &small(Background::Color([20, 40, 200]))).unwrap();
    assert_eq!(pixel(&blue, 0, 0), [20, 40, 200, 255]);
    let clear = render_presentation(&box_on_slab(), &small(Background::Transparent)).unwrap();
    assert_eq!(pixel(&clear, 0, 0)[3], 0);
    assert!(pixels(&clear).any(|p| p[3] == 255), "the model is opaque");
}

#[test]
fn the_model_casts_a_soft_shadow_on_the_ground_under_it() {
    let options = PresentationOptions {
        supersample: 1,
        ambient_occlusion: false,
        outlines: false,
        ..small(Background::Transparent)
    };
    let standing = PresentationScene {
        triangles: cuboid([-50.0, -50.0, 0.0], [50.0, 50.0, 120.0], [200, 60, 50, 255]),
    };
    let shadowed = render_presentation(&standing, &options).unwrap();
    let partial =
        |image: &PresentationImage| pixels(image).filter(|p| p[3] > 0 && p[3] < 255).count();
    // Without smoothing only the shadow on the implied ground is partly transparent.
    assert!(
        partial(&shadowed) > 100,
        "{} shadow pixels",
        partial(&shadowed)
    );
    assert!(pixels(&shadowed).all(|p| p[3] == 255 || p[0..3] == [0, 0, 0]));
    let unshadowed = render_presentation(
        &standing,
        &PresentationOptions {
            shadows: false,
            ..options
        },
    )
    .unwrap();
    assert_eq!(partial(&unshadowed), 0);
    // The box shades part of the slab too.
    let lit = render_presentation(
        &box_on_slab(),
        &PresentationOptions {
            shadows: false,
            ..small(Background::White)
        },
    )
    .unwrap();
    let dark = render_presentation(&box_on_slab(), &small(Background::White)).unwrap();
    assert!(brightness(&dark) < brightness(&lit));
}

#[test]
fn corners_where_surfaces_meet_are_darker_with_ambient_occlusion() {
    let base = PresentationOptions {
        shadows: false,
        outlines: false,
        ..small(Background::White)
    };
    let open = render_presentation(
        &box_on_slab(),
        &PresentationOptions {
            ambient_occlusion: false,
            ..base
        },
    )
    .unwrap();
    let occluded = render_presentation(&box_on_slab(), &base).unwrap();
    let darker = open
        .rgba
        .chunks_exact(4)
        .zip(occluded.rgba.chunks_exact(4))
        .filter(|(a, b)| u32::from(a[0]) > u32::from(b[0]) + 10)
        .count();
    let most = open
        .rgba
        .chunks_exact(4)
        .zip(occluded.rgba.chunks_exact(4))
        .map(|(a, b)| i32::from(a[0]) - i32::from(b[0]))
        .max();
    assert!(darker > 20, "{darker} pixels darkened, most {most:?}");
    // Only near the corner: most of the model keeps its light.
    assert!(darker * 4 < open.rgba.len() / 4);
}

#[test]
fn a_see_through_part_shows_what_is_behind_it() {
    // A blue membrane between the viewer and the box, inside the slab's and box's extent so the
    // camera frames all three scenes alike.
    let scene = |alpha: Option<u8>| {
        let mut scene = box_on_slab();
        if let Some(alpha) = alpha {
            let rgba = [40, 80, 220, alpha];
            let [a, b, c, d] = [
                [-150.0, -120.0, 20.0],
                [150.0, -120.0, 20.0],
                [150.0, -120.0, 140.0],
                [-150.0, -120.0, 140.0],
            ];
            scene.triangles.extend([
                PresentationTriangle {
                    corners_mm: [a, b, c],
                    rgba,
                },
                PresentationTriangle {
                    corners_mm: [a, c, d],
                    rgba,
                },
            ]);
        }
        scene
    };
    let options = PresentationOptions {
        shadows: false,
        ambient_occlusion: false,
        outlines: false,
        ..small(Background::White)
    };
    let behind = render_presentation(&scene(None), &options).unwrap();
    let opaque = render_presentation(&scene(Some(255)), &options).unwrap();
    let membrane = render_presentation(&scene(Some(110)), &options).unwrap();
    let alpha = 110.0 / 255.0;
    let mut covered = 0;
    let mut error = 0.0;
    for ((back, front), mixed) in behind
        .rgba
        .chunks_exact(4)
        .zip(opaque.rgba.chunks_exact(4))
        .zip(membrane.rgba.chunks_exact(4))
    {
        if (0..3).all(|c| back[c].abs_diff(front[c]) < 30) {
            continue;
        }
        covered += 1;
        assert_eq!(mixed[3], 255);
        for c in 0..3 {
            let expected = f64::from(front[c]) * alpha + f64::from(back[c]) * (1.0 - alpha);
            error += (f64::from(mixed[c]) - expected).abs();
        }
    }
    assert!(covered > 500, "the membrane covers {covered} pixels");
    let mean = error / f64::from(covered * 3);
    assert!(mean < 2.0, "mean departure from the blend {mean}");
}

#[test]
fn supersampling_smooths_the_outline_against_the_background() {
    let options = PresentationOptions {
        shadows: false,
        ..small(Background::Transparent)
    };
    let partial = |supersample: u32| {
        let image = render_presentation(
            &box_on_slab(),
            &PresentationOptions {
                supersample,
                ..options
            },
        )
        .unwrap();
        pixels(&image).filter(|p| p[3] > 0 && p[3] < 255).count()
    };
    assert_eq!(partial(1), 0);
    assert!(partial(3) > 50);
}

#[test]
fn invalid_requests_are_refused_with_their_reason() {
    let scene = box_on_slab();
    let refuse = |options: PresentationOptions| render_presentation(&scene, &options).unwrap_err();
    assert!(matches!(
        refuse(PresentationOptions {
            width: 0,
            ..small(Background::White)
        }),
        PresentationError::Size { width: 0, .. }
    ));
    assert!(matches!(
        refuse(PresentationOptions {
            supersample: 5,
            ..small(Background::White)
        }),
        PresentationError::Supersample { supersample: 5 }
    ));
    assert!(matches!(
        refuse(PresentationOptions {
            width: 8000,
            height: 8000,
            ..small(Background::White)
        }),
        PresentationError::TooManySamples { .. }
    ));
    assert!(matches!(
        refuse(PresentationOptions {
            view_from: [0.0; 3],
            ..small(Background::White)
        }),
        PresentationError::ViewDirection
    ));
    assert!(matches!(
        render_presentation(&PresentationScene::default(), &small(Background::White)),
        Err(PresentationError::EmptyModel)
    ));
    let mut broken = box_on_slab();
    broken.triangles[3].corners_mm[1][2] = f64::NAN;
    assert!(matches!(
        render_presentation(&broken, &small(Background::White)),
        Err(PresentationError::NonFiniteGeometry { triangle: 3 })
    ));
}

#[test]
fn a_model_renders_with_its_part_colours_and_chosen_opacity() {
    let document = seeded_document(Some([220, 30, 30]), None);
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
    let scene = presentation_scene(&snapshot, &instances, |occurrence| {
        if occurrence.color.is_some() { 255 } else { 120 }
    })
    .unwrap();
    assert_eq!(
        scene.triangles.len(),
        package.triangles().len() * occurrences.len()
    );
    assert!(scene.triangles.iter().any(|t| t.rgba == [220, 30, 30, 255]));
    assert!(scene.triangles.iter().any(|t| t.rgba[3] == 120));
    let image = render_presentation(&scene, &small(Background::White)).unwrap();
    assert!(pixels(&image).any(|p| p[0] > 120 && p[1] < 60 && p[2] < 60));
}
