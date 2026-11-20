//! Each refused surface or loft input names its own field (R1120ac-EXA-901).
use super::*;

fn refused_field(result: Result<(), CanonicalError>) -> FeatureInputField {
    match result {
        Err(CanonicalError::InvalidFeatureInput(error)) => {
            assert!(error.to_string().starts_with("feature input "), "{error}");
            error.field
        }
        other => panic!("expected a refused feature input, got {other:?}"),
    }
}

fn control_point(offset_mm: [f64; 3], weight: Option<f64>, keep: &[u64]) -> SurfaceBodySpec {
    SurfaceBodySpec::ControlPoint {
        target: FeatureId(620),
        u_index: 1,
        v_index: 1,
        from_mm: [0.0; 3],
        offset_mm,
        weight,
        keep: keep
            .iter()
            .map(|id| SurfaceSeam {
                surface: FeatureId(*id),
                continuity: LoftContinuity::Tangent,
            })
            .collect(),
    }
}

#[test]
fn a_control_point_move_names_the_weight_offset_or_seam_not_the_target() {
    let up = [0.0, 0.0, 1.0];
    assert_eq!(surface_body(&control_point(up, None, &[621]), 1e-6), Ok(()));
    for weight in [1e-6, 2.5, 1e6] {
        assert_eq!(
            surface_body(&control_point(up, Some(weight), &[]), 1e-6),
            Ok(())
        );
    }
    for (spec, field) in [
        (control_point(up, Some(0.0), &[]), FeatureInputField::Weight),
        (
            control_point(up, Some(-1.0), &[]),
            FeatureInputField::Weight,
        ),
        // Unbounded weights were taken (review 2026-11-20 ac P3).
        (
            control_point(up, Some(f64::MAX), &[]),
            FeatureInputField::Weight,
        ),
        (
            control_point(up, Some(1e-300), &[]),
            FeatureInputField::Weight,
        ),
        (
            control_point(up, Some(f64::NAN), &[]),
            FeatureInputField::Weight,
        ),
        (
            control_point([0.0; 3], None, &[]),
            FeatureInputField::OffsetMm,
        ),
        (
            control_point([f64::NAN, 0.0, 0.0], None, &[]),
            FeatureInputField::OffsetMm,
        ),
        (
            control_point(up, None, &[620]),
            FeatureInputField::KeepSeam(0),
        ),
        (
            control_point(up, None, &[621, 621]),
            FeatureInputField::KeepSeam(1),
        ),
    ] {
        assert_eq!(refused_field(surface_body(&spec, 1e-6)), field);
    }
    let nan_from = SurfaceBodySpec::ControlPoint {
        target: FeatureId(620),
        u_index: 1,
        v_index: 1,
        from_mm: [0.0, f64::NAN, 0.0],
        offset_mm: up,
        weight: None,
        keep: Vec::new(),
    };
    assert_eq!(
        refused_field(surface_body(&nan_from, 1e-6)),
        FeatureInputField::FromMm
    );
}

#[test]
fn a_blend_trim_or_boundary_names_its_own_field() {
    let blend = SurfaceBodySpec::Blend {
        first: FeatureId(620),
        first_continuity: LoftContinuity::Tangent,
        second: FeatureId(620),
        second_continuity: LoftContinuity::Tangent,
    };
    assert_eq!(
        refused_field(surface_body(&blend, 1e-6)),
        FeatureInputField::Second
    );
    assert_eq!(
        refused_field(surface_trim(FeatureId(620), FeatureId(620))),
        FeatureInputField::Cutter
    );
    let boundary = SurfaceBodySpec::Boundary {
        segments: Vec::new(),
        inner: Vec::new(),
    };
    let refused = surface_body(&boundary, 1e-6).unwrap_err();
    assert_eq!(
        refused_field(Err(refused.clone())),
        FeatureInputField::Segments
    );
    assert_ne!(refused.code(), CanonicalError::InvalidSweep.code());
}

#[test]
fn a_loft_names_the_section_guide_rail_or_continuity() {
    let section = |profile, elevation_mm| LoftSection {
        profile: FeatureId(profile),
        elevation_mm,
    };
    let rising = [section(1, 0.0), section(2, 10.0)];
    let tangent = LoftContinuity::Tangent;
    let guide = Some(FeatureId(9));
    assert_eq!(loft(&rising, guide, tangent, Some(FeatureId(8))), Ok(()));
    for (result, field) in [
        (
            loft(&rising[..1], None, tangent, None),
            FeatureInputField::Sections,
        ),
        (
            loft(&rising, guide, LoftContinuity::Curvature, None),
            FeatureInputField::Continuity,
        ),
        (
            loft(&rising, None, tangent, Some(FeatureId(8))),
            FeatureInputField::Rail,
        ),
        (
            loft(&rising, guide, tangent, guide),
            FeatureInputField::Rail,
        ),
        (
            loft(&[section(1, 10.0), section(2, 0.0)], None, tangent, None),
            FeatureInputField::SectionElevation(1),
        ),
        (
            loft(
                &[section(1, 0.0), section(2, f64::INFINITY)],
                None,
                tangent,
                None,
            ),
            FeatureInputField::SectionElevation(1),
        ),
        (
            loft(&[section(1, 0.0), section(1, 10.0)], None, tangent, None),
            FeatureInputField::SectionProfile(1),
        ),
        (loft_profile(2, false), FeatureInputField::SectionProfile(2)),
        (
            loft_path(FeatureInputField::Guide, tangent, false),
            FeatureInputField::Guide,
        ),
        (
            loft_path(FeatureInputField::Rail, tangent, false),
            FeatureInputField::Rail,
        ),
    ] {
        assert_eq!(refused_field(result), field);
    }
    assert_eq!(
        CanonicalError::InvalidFeatureInput(FeatureInputError {
            field: FeatureInputField::KeepSeam(3),
            rule: "this surface is already kept; name each neighbour once",
        })
        .to_string(),
        "feature input keep[3] is invalid: this surface is already kept; name each neighbour once"
    );
}
