use super::blender_glb_export::{current_package, seeded_document};
use ketchup_manufacturing::blender_export::MeshGlbInstance;
use ketchup_manufacturing::presentation::{
    Background, MAX_LABEL_CHARS, MAX_PRESENTATION_LABELS, PresentationError, PresentationImage,
    PresentationLabel, PresentationOptions, PresentationScene, PresentationTexture,
    PresentationTriangle, TextureError, TriangleTexture, part_labels, presentation_scene,
    render_presentation,
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
                    texture: None,
                },
                PresentationTriangle {
                    corners_mm: [corner(*a), corner(*c), corner(*d)],
                    rgba,
                    texture: None,
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
    PresentationScene {
        triangles,
        labels: Vec::new(),
        textures: Vec::new(),
    }
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
        labels: Vec::new(),
        textures: Vec::new(),
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
                    texture: None,
                },
                PresentationTriangle {
                    corners_mm: [a, c, d],
                    rgba,
                    texture: None,
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
    let scene = presentation_scene(&snapshot, &instances, &Default::default(), |occurrence| {
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

/// Plain shading at a size where text has room.
fn annotated(background: Background) -> PresentationOptions {
    PresentationOptions {
        width: 320,
        height: 240,
        shadows: false,
        ambient_occlusion: false,
        outlines: false,
        ..small(background)
    }
}

/// Dark grey pixels: the ink of dimensions and labels, not a shaded part.
fn ink(image: &PresentationImage) -> usize {
    pixels(image)
        .filter(|p| {
            let (low, high) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
            p[3] > 0 && high < 70 && high - low < 12
        })
        .count()
}

fn changed(a: &PresentationImage, b: &PresentationImage) -> usize {
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count()
}

#[test]
fn dimensions_draw_the_model_extents_and_the_drawing_stays_in_frame() {
    let plain = annotated(Background::White);
    let options = PresentationOptions {
        dimensions: true,
        ..plain
    };
    let without = render_presentation(&box_on_slab(), &plain).unwrap();
    let with = render_presentation(&box_on_slab(), &options).unwrap();
    assert_eq!(with, render_presentation(&box_on_slab(), &options).unwrap());
    assert!(
        ink(&with) > ink(&without) + 150,
        "{} ink pixels with dimensions, {} without",
        ink(&with),
        ink(&without)
    );
    // The camera frames the dimensions with the model, so nothing touches the image edge.
    let (width, height) = (with.width, with.height);
    for x in 0..width {
        assert_eq!(pixel(&with, x, 0), [255; 4]);
        assert_eq!(pixel(&with, x, height - 1), [255; 4]);
    }
    for y in 0..height {
        assert_eq!(pixel(&with, 0, y), [255; 4]);
        assert_eq!(pixel(&with, width - 1, y), [255; 4]);
    }
    // Over a transparent background the ink is opaque where it is drawn.
    let clear = render_presentation(
        &box_on_slab(),
        &PresentationOptions {
            background: Background::Transparent,
            ..options
        },
    )
    .unwrap();
    assert!(ink(&clear) > 150);
}

#[test]
fn a_label_names_a_point_with_a_leader_line_and_its_text() {
    let options = annotated(Background::White);
    let labelled = |text: &str| {
        let mut scene = box_on_slab();
        scene.labels.push(PresentationLabel {
            text: text.into(),
            at_mm: [50.0, -50.0, 140.0],
        });
        scene
    };
    let plain = render_presentation(&box_on_slab(), &options).unwrap();
    let short = render_presentation(&labelled("A"), &options).unwrap();
    let long = render_presentation(&labelled("Membrána 2 mm"), &options).unwrap();
    assert_eq!(
        long,
        render_presentation(&labelled("Membrána 2 mm"), &options).unwrap()
    );
    assert!(changed(&plain, &short) > 20);
    assert!(
        changed(&plain, &long) > changed(&plain, &short) + 50,
        "longer text covers more: {} against {}",
        changed(&plain, &long),
        changed(&plain, &short)
    );
    // Thin letters at this size are mostly smoothed grey: count what the label made darker.
    let darkened = plain
        .rgba
        .chunks_exact(4)
        .zip(long.rgba.chunks_exact(4))
        .filter(|(before, after)| {
            (0..3)
                .map(|c| i32::from(before[c]) - i32::from(after[c]))
                .sum::<i32>()
                > 150
        })
        .count();
    assert!(darkened > 40, "{darkened} pixels darkened by the label");
}

#[test]
fn labels_without_text_or_a_finite_point_or_too_many_are_refused() {
    let refuse = |labels: Vec<PresentationLabel>| {
        let mut scene = box_on_slab();
        scene.labels = labels;
        render_presentation(&scene, &annotated(Background::White)).unwrap_err()
    };
    let label = |text: &str| PresentationLabel {
        text: text.into(),
        at_mm: [0.0, 0.0, 0.0],
    };
    for text in [
        "  ".to_owned(),
        "a\nb".to_owned(),
        "x".repeat(MAX_LABEL_CHARS + 1),
    ] {
        assert!(
            matches!(
                refuse(vec![label("fine"), label(&text)]),
                PresentationError::LabelText { label: 1, .. }
            ),
            "{text:?}"
        );
    }
    assert!(matches!(
        refuse(vec![PresentationLabel {
            text: "far".into(),
            at_mm: [0.0, f64::INFINITY, 0.0],
        }]),
        PresentationError::LabelPoint { label: 0 }
    ));
    assert!(matches!(
        refuse(vec![label("many"); MAX_PRESENTATION_LABELS + 1]),
        PresentationError::TooManyLabels { .. }
    ));
}

#[test]
fn part_labels_name_each_component_once_on_top_of_its_first_part() {
    let document = seeded_document(None, None);
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
    let labels = part_labels(&instances);
    assert_eq!(
        labels,
        vec![PresentationLabel {
            text: "Reusable beam".into(),
            at_mm: [5.0, 5.0, 10.0],
        }]
    );
}

/// A 2 × 2 PNG: red and white squares.
fn checker_png() -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[200, 30, 30, 255, 255, 255, 255, 255, 255, 200, 30, 30])
        .unwrap();
    writer.finish().unwrap();
    bytes
}

/// A textured face shows its picture, repeating as its texture coordinates say, where an
/// untextured one shows a single colour (U4e).
#[test]
fn a_textured_face_shows_its_picture_repeating() {
    // A 400 × 400 mm wall facing the viewer, the picture four times along each side.
    let [a, b, c, d] = [
        [-200.0, 0.0, 0.0],
        [200.0, 0.0, 0.0],
        [200.0, 0.0, 400.0],
        [-200.0, 0.0, 400.0],
    ];
    let [ua, ub, uc, ud] = [[0.0, 4.0], [4.0, 4.0], [4.0, 0.0], [0.0, 0.0]];
    let wall = |textured: bool| {
        let texture = |uv| textured.then_some(TriangleTexture { texture: 0, uv });
        PresentationScene {
            triangles: vec![
                PresentationTriangle {
                    corners_mm: [a, b, c],
                    rgba: [150, 150, 150, 255],
                    texture: texture([ua, ub, uc]),
                },
                PresentationTriangle {
                    corners_mm: [a, c, d],
                    rgba: [150, 150, 150, 255],
                    texture: texture([ua, uc, ud]),
                },
            ],
            labels: Vec::new(),
            textures: vec![PresentationTexture::decode(&checker_png(), "image/png").unwrap()],
        }
    };
    let options = PresentationOptions {
        view_from: [0.0, -1.0, 0.0],
        shadows: false,
        ambient_occlusion: false,
        outlines: false,
        ..small(Background::Transparent)
    };
    let plain = render_presentation(&wall(false), &options).unwrap();
    let textured = render_presentation(&wall(true), &options).unwrap();
    assert_eq!(
        textured,
        render_presentation(&wall(true), &options).unwrap()
    );
    let ratios = |image: &PresentationImage| {
        pixels(image)
            .filter(|p| p[3] == 255)
            .map(|p| f64::from(p[1]) / f64::from(p[0]))
            .collect::<Vec<_>>()
    };
    let flat = ratios(&plain);
    assert!(
        flat.iter().all(|ratio| (ratio - 1.0).abs() < 0.02),
        "one grey"
    );
    let shown = ratios(&textured);
    let red = shown.iter().filter(|ratio| **ratio < 0.6).count();
    let white = shown.iter().filter(|ratio| **ratio > 0.8).count();
    assert!(
        red * 20 > shown.len() && white * 20 > shown.len(),
        "{red} red and {white} white of {}",
        shown.len()
    );
    // Along a row through the centres of a row of squares (5/16 down the wall) the colour
    // turns twice per 100 mm repeat: 7 or 8 times across the 4 repeats.
    let rows = (0..textured.height)
        .filter(|y| (0..textured.width).any(|x| pixel(&textured, x, *y)[3] == 255))
        .collect::<Vec<_>>();
    let (top, bottom) = (rows[0], rows[rows.len() - 1]);
    let row = top + (bottom - top) * 5 / 16;
    let reds = (0..textured.width)
        .map(|x| pixel(&textured, x, row))
        .filter(|p| p[3] == 255)
        .map(|p| p[1] < 120)
        .collect::<Vec<_>>();
    let turns = reds.windows(2).filter(|pair| pair[0] != pair[1]).count();
    assert!((7..=9).contains(&turns), "{turns} turns along {reds:?}");

    let mut broken = wall(true);
    broken.textures.clear();
    assert!(matches!(
        render_presentation(&broken, &options),
        Err(PresentationError::TextureReference { triangle: 0 })
    ));
    let not_png = PresentationTexture::decode(b"not a picture", "image/png");
    assert!(matches!(&not_png, Err(TextureError::Png(_))), "{not_png:?}");
    assert!(std::error::Error::source(&not_png.unwrap_err()).is_some());
    assert!(matches!(
        PresentationTexture::decode(&checker_png(), "image/gif"),
        Err(TextureError::MediaType(_))
    ));
}

/// A picture stands the right way up: texture coordinates follow glTF, the top row at v = 0.
#[test]
fn a_picture_on_a_face_stands_the_right_way_up() {
    let mut png_bytes = Vec::new();
    // Red top left, green top right, blue bottom left, white bottom right.
    let mut encoder = png::Encoder::new(&mut png_bytes, 2, 2);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer
        .write_image_data(&[200, 30, 30, 30, 200, 30, 30, 30, 200, 255, 255, 255])
        .unwrap();
    writer.finish().unwrap();
    // A 400 × 400 mm wall facing the viewer, v = 0 along its top edge.
    let [a, b, c, d] = [
        [-200.0, 0.0, 0.0],
        [200.0, 0.0, 0.0],
        [200.0, 0.0, 400.0],
        [-200.0, 0.0, 400.0],
    ];
    let [ua, ub, uc, ud] = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];
    let triangle = |corners_mm, uv| PresentationTriangle {
        corners_mm,
        rgba: [150, 150, 150, 255],
        texture: Some(TriangleTexture { texture: 0, uv }),
    };
    let scene = PresentationScene {
        triangles: vec![
            triangle([a, b, c], [ua, ub, uc]),
            triangle([a, c, d], [ua, uc, ud]),
        ],
        labels: Vec::new(),
        textures: vec![PresentationTexture::decode(&png_bytes, "image/png").unwrap()],
    };
    let options = PresentationOptions {
        view_from: [0.0, -1.0, 0.0],
        shadows: false,
        ambient_occlusion: false,
        outlines: false,
        ..small(Background::Transparent)
    };
    let image = render_presentation(&scene, &options).unwrap();
    let rows = (0..image.height)
        .filter(|y| (0..image.width).any(|x| pixel(&image, x, *y)[3] == 255))
        .collect::<Vec<_>>();
    // The centre of each texel: a quarter and three quarters across and down the face.
    let at = |y: u32, across: usize| {
        let xs = (0..image.width)
            .filter(|x| pixel(&image, *x, y)[3] == 255)
            .collect::<Vec<_>>();
        pixel(&image, xs[xs.len() * across / 4], y)
    };
    let (top, bottom) = (rows[rows.len() / 4], rows[rows.len() * 3 / 4]);
    // Shading scales the colour, so name each by its strongest channel, or white when even.
    let colour = |p: [u8; 4]| {
        let [r, g, b] = [p[0], p[1], p[2]].map(f64::from);
        if r.min(g).min(b) > 0.8 * r.max(g).max(b) {
            "white"
        } else if r > g && r > b {
            "red"
        } else if g > b {
            "green"
        } else {
            "blue"
        }
    };
    let seen = [(top, 1), (top, 3), (bottom, 1), (bottom, 3)].map(|(y, x)| colour(at(y, x)));
    assert_eq!(seen, ["red", "green", "blue", "white"]);
    // From behind the picture shows through mirrored, as SketchUp shows it.
    let behind = PresentationOptions {
        view_from: [0.0, 1.0, 0.0],
        ..options
    };
    let image = render_presentation(&scene, &behind).unwrap();
    let at = |y: u32, across: usize| {
        let xs = (0..image.width)
            .filter(|x| pixel(&image, *x, y)[3] == 255)
            .collect::<Vec<_>>();
        pixel(&image, xs[xs.len() * across / 4], y)
    };
    let seen = [(top, 1), (top, 3), (bottom, 1), (bottom, 3)].map(|(y, x)| colour(at(y, x)));
    assert_eq!(seen, ["green", "red", "white", "blue"]);
}
