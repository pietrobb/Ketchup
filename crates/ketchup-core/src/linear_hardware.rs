const INSTALLATION_TOLERANCE_MM: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRunnerCatalogSpec {
    manufacturer: &'static str,
    family: &'static str,
    article_number: &'static str,
    source_document: &'static str,
    source_sha256: &'static str,
    cad_catalog_identity: &'static str,
    nominal_length_mm: f64,
    minimum_cabinet_depth_mm: f64,
    installed_width_per_side_mm: f64,
    maximum_drawer_side_thickness_mm: f64,
    mounting_hole_spacing_mm: f64,
    maximum_travel_mm: f64,
}

impl LinearRunnerCatalogSpec {
    #[must_use]
    pub const fn manufacturer(self) -> &'static str {
        self.manufacturer
    }

    #[must_use]
    pub const fn family(self) -> &'static str {
        self.family
    }

    #[must_use]
    pub const fn article_number(self) -> &'static str {
        self.article_number
    }

    #[must_use]
    pub const fn source_document(self) -> &'static str {
        self.source_document
    }

    #[must_use]
    pub const fn source_sha256(self) -> &'static str {
        self.source_sha256
    }

    #[must_use]
    pub const fn cad_catalog_identity(self) -> &'static str {
        self.cad_catalog_identity
    }

    #[must_use]
    pub const fn nominal_length_mm(self) -> f64 {
        self.nominal_length_mm
    }

    #[must_use]
    pub const fn minimum_cabinet_depth_mm(self) -> f64 {
        self.minimum_cabinet_depth_mm
    }

    #[must_use]
    pub const fn installed_width_per_side_mm(self) -> f64 {
        self.installed_width_per_side_mm
    }

    #[must_use]
    pub const fn maximum_drawer_side_thickness_mm(self) -> f64 {
        self.maximum_drawer_side_thickness_mm
    }

    #[must_use]
    pub const fn mounting_hole_spacing_mm(self) -> f64 {
        self.mounting_hole_spacing_mm
    }

    #[must_use]
    pub const fn maximum_travel_mm(self) -> f64 {
        self.maximum_travel_mm
    }

    #[must_use]
    pub fn required_drawer_outer_width_mm(self, cabinet_inner_width_mm: f64) -> Option<f64> {
        let width = cabinet_inner_width_mm - 2.0 * self.installed_width_per_side_mm;
        (cabinet_inner_width_mm.is_finite() && width > 0.0).then_some(width)
    }

    fn is_valid(self) -> bool {
        !self.manufacturer.is_empty()
            && !self.family.is_empty()
            && !self.article_number.is_empty()
            && !self.source_document.is_empty()
            && self.source_sha256.len() == 64
            && self
                .source_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            && !self.cad_catalog_identity.is_empty()
            && self.nominal_length_mm.is_finite()
            && self.nominal_length_mm > 0.0
            && self.minimum_cabinet_depth_mm.is_finite()
            && self.minimum_cabinet_depth_mm >= self.nominal_length_mm
            && self.installed_width_per_side_mm.is_finite()
            && self.installed_width_per_side_mm > 0.0
            && self.maximum_drawer_side_thickness_mm.is_finite()
            && self.maximum_drawer_side_thickness_mm > 0.0
            && self.mounting_hole_spacing_mm.is_finite()
            && self.mounting_hole_spacing_mm > 0.0
            && self.mounting_hole_spacing_mm < self.nominal_length_mm
            && self.maximum_travel_mm.is_finite()
            && self.maximum_travel_mm > 0.0
            && self.maximum_travel_mm <= self.nominal_length_mm
    }
}

pub const HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450: LinearRunnerCatalogSpec =
    LinearRunnerCatalogSpec {
        manufacturer: "Hettich",
        family: "Quadro V6 Push to open, slide-on, EB 20",
        article_number: "9135991",
        source_document: "MTA_929680100_QV6_SFP_PTO_EB20",
        source_sha256: "e582546768e8757c6de6ba63e02b5cff503b7b9a540f64777ebcc984f3db5e05",
        cad_catalog_identity: "hettich/drawer_runners/quadro/slide_on/quadro_v6_push_open/push_open_asmtab.prj;LINEID=520;NB=9135991;NL=450;H=256;D=463;C=319.5;EB=20",
        nominal_length_mm: 450.0,
        minimum_cabinet_depth_mm: 463.0,
        installed_width_per_side_mm: 20.0,
        maximum_drawer_side_thickness_mm: 16.0,
        mounting_hole_spacing_mm: 256.0,
        maximum_travel_mm: 450.0,
    };

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearRunnerInstallation {
    pub cabinet_inner_width_mm: f64,
    pub cabinet_depth_mm: f64,
    pub drawer_outer_width_mm: f64,
    pub drawer_side_thickness_mm: f64,
    pub left_runner_length_mm: f64,
    pub right_runner_length_mm: f64,
    pub left_runner_height_mm: f64,
    pub right_runner_height_mm: f64,
    pub requested_travel_mm: f64,
}

impl LinearRunnerInstallation {
    fn is_valid(self) -> bool {
        [
            self.cabinet_inner_width_mm,
            self.cabinet_depth_mm,
            self.drawer_outer_width_mm,
            self.drawer_side_thickness_mm,
            self.left_runner_length_mm,
            self.right_runner_length_mm,
        ]
        .into_iter()
        .all(|value| value.is_finite() && value >= 0.0)
            && self.cabinet_inner_width_mm > 0.0
            && self.cabinet_depth_mm > 0.0
            && self.drawer_outer_width_mm > 0.0
            && self.drawer_side_thickness_mm > 0.0
            && self.left_runner_length_mm > 0.0
            && self.right_runner_length_mm > 0.0
            && self.requested_travel_mm.is_finite()
            && self.left_runner_height_mm.is_finite()
            && self.right_runner_height_mm.is_finite()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinearRunnerSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinearRunnerInstallationIssue {
    CabinetDepthBelowMinimum {
        actual_mm: f64,
        minimum_mm: f64,
    },
    DrawerWidthMismatch {
        actual_mm: f64,
        required_mm: f64,
        tolerance_mm: f64,
    },
    DrawerSideTooThick {
        actual_mm: f64,
        maximum_mm: f64,
    },
    RunnerNominalLengthMismatch {
        side: LinearRunnerSide,
        actual_mm: f64,
        required_mm: f64,
        tolerance_mm: f64,
    },
    RunnerPairLengthMismatch {
        mismatch_mm: f64,
        tolerance_mm: f64,
    },
    RunnerPairHeightMismatch {
        mismatch_mm: f64,
        tolerance_mm: f64,
    },
    TravelOutsideLimits {
        actual_mm: f64,
        minimum_mm: f64,
        maximum_mm: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LinearRunnerInstallationReport {
    required_drawer_outer_width_mm: f64,
    issues: Vec<LinearRunnerInstallationIssue>,
}

impl LinearRunnerInstallationReport {
    #[must_use]
    pub const fn required_drawer_outer_width_mm(&self) -> f64 {
        self.required_drawer_outer_width_mm
    }

    #[must_use]
    pub fn issues(&self) -> &[LinearRunnerInstallationIssue] {
        &self.issues
    }

    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinearRunnerInstallationError {
    InvalidCatalogSpec,
    InvalidInstallation,
}

pub fn validate_linear_runner_installation(
    spec: LinearRunnerCatalogSpec,
    installation: LinearRunnerInstallation,
) -> Result<LinearRunnerInstallationReport, LinearRunnerInstallationError> {
    if !spec.is_valid() {
        return Err(LinearRunnerInstallationError::InvalidCatalogSpec);
    }
    if !installation.is_valid() {
        return Err(LinearRunnerInstallationError::InvalidInstallation);
    }
    let required_drawer_outer_width_mm = spec
        .required_drawer_outer_width_mm(installation.cabinet_inner_width_mm)
        .ok_or(LinearRunnerInstallationError::InvalidInstallation)?;
    let mut issues = Vec::new();

    if installation.cabinet_depth_mm < spec.minimum_cabinet_depth_mm {
        issues.push(LinearRunnerInstallationIssue::CabinetDepthBelowMinimum {
            actual_mm: installation.cabinet_depth_mm,
            minimum_mm: spec.minimum_cabinet_depth_mm,
        });
    }
    if (installation.drawer_outer_width_mm - required_drawer_outer_width_mm).abs()
        > INSTALLATION_TOLERANCE_MM
    {
        issues.push(LinearRunnerInstallationIssue::DrawerWidthMismatch {
            actual_mm: installation.drawer_outer_width_mm,
            required_mm: required_drawer_outer_width_mm,
            tolerance_mm: INSTALLATION_TOLERANCE_MM,
        });
    }
    if installation.drawer_side_thickness_mm > spec.maximum_drawer_side_thickness_mm {
        issues.push(LinearRunnerInstallationIssue::DrawerSideTooThick {
            actual_mm: installation.drawer_side_thickness_mm,
            maximum_mm: spec.maximum_drawer_side_thickness_mm,
        });
    }
    for (side, actual_mm) in [
        (LinearRunnerSide::Left, installation.left_runner_length_mm),
        (LinearRunnerSide::Right, installation.right_runner_length_mm),
    ] {
        if (actual_mm - spec.nominal_length_mm).abs() > INSTALLATION_TOLERANCE_MM {
            issues.push(LinearRunnerInstallationIssue::RunnerNominalLengthMismatch {
                side,
                actual_mm,
                required_mm: spec.nominal_length_mm,
                tolerance_mm: INSTALLATION_TOLERANCE_MM,
            });
        }
    }
    let pair_length_mismatch_mm =
        (installation.left_runner_length_mm - installation.right_runner_length_mm).abs();
    if pair_length_mismatch_mm > INSTALLATION_TOLERANCE_MM {
        issues.push(LinearRunnerInstallationIssue::RunnerPairLengthMismatch {
            mismatch_mm: pair_length_mismatch_mm,
            tolerance_mm: INSTALLATION_TOLERANCE_MM,
        });
    }
    let pair_height_mismatch_mm =
        (installation.left_runner_height_mm - installation.right_runner_height_mm).abs();
    if pair_height_mismatch_mm > INSTALLATION_TOLERANCE_MM {
        issues.push(LinearRunnerInstallationIssue::RunnerPairHeightMismatch {
            mismatch_mm: pair_height_mismatch_mm,
            tolerance_mm: INSTALLATION_TOLERANCE_MM,
        });
    }
    if installation.requested_travel_mm < 0.0
        || installation.requested_travel_mm > spec.maximum_travel_mm
    {
        issues.push(LinearRunnerInstallationIssue::TravelOutsideLimits {
            actual_mm: installation.requested_travel_mm,
            minimum_mm: 0.0,
            maximum_mm: spec.maximum_travel_mm,
        });
    }

    Ok(LinearRunnerInstallationReport {
        required_drawer_outer_width_mm,
        issues,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_installation() -> LinearRunnerInstallation {
        LinearRunnerInstallation {
            cabinet_inner_width_mm: 600.0,
            cabinet_depth_mm: 463.0,
            drawer_outer_width_mm: 560.0,
            drawer_side_thickness_mm: 16.0,
            left_runner_length_mm: 450.0,
            right_runner_length_mm: 450.0,
            left_runner_height_mm: 100.0,
            right_runner_height_mm: 100.0,
            requested_travel_mm: 450.0,
        }
    }

    #[test]
    fn hettich_quadro_v6_450_profile_matches_the_published_mounting_contract() {
        let spec = HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450;
        assert_eq!(spec.manufacturer(), "Hettich");
        assert_eq!(spec.article_number(), "9135991");
        assert_eq!(spec.source_document(), "MTA_929680100_QV6_SFP_PTO_EB20");
        assert_eq!(
            spec.source_sha256(),
            "e582546768e8757c6de6ba63e02b5cff503b7b9a540f64777ebcc984f3db5e05"
        );
        assert!(spec.cad_catalog_identity().contains("NB=9135991"));
        assert!(spec.cad_catalog_identity().contains("NL=450"));
        assert_eq!(spec.nominal_length_mm(), 450.0);
        assert_eq!(spec.minimum_cabinet_depth_mm(), 463.0);
        assert_eq!(spec.installed_width_per_side_mm(), 20.0);
        assert_eq!(spec.maximum_drawer_side_thickness_mm(), 16.0);
        assert_eq!(spec.mounting_hole_spacing_mm(), 256.0);
        assert_eq!(spec.maximum_travel_mm(), 450.0);
        assert_eq!(spec.required_drawer_outer_width_mm(600.0), Some(560.0));

        let report = validate_linear_runner_installation(spec, valid_installation()).unwrap();
        assert!(report.is_valid());
        assert_eq!(report.required_drawer_outer_width_mm(), 560.0);
        assert!(report.issues().is_empty());
    }

    #[test]
    fn invalid_installation_reports_every_actionable_fit_and_motion_problem() {
        let report = validate_linear_runner_installation(
            HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450,
            LinearRunnerInstallation {
                cabinet_inner_width_mm: 600.0,
                cabinet_depth_mm: 462.0,
                drawer_outer_width_mm: 562.0,
                drawer_side_thickness_mm: 17.0,
                left_runner_length_mm: 450.0,
                right_runner_length_mm: 448.0,
                left_runner_height_mm: 100.0,
                right_runner_height_mm: 102.0,
                requested_travel_mm: 451.0,
            },
        )
        .unwrap();

        assert!(!report.is_valid());
        assert_eq!(report.issues().len(), 7);
        assert!(matches!(
            report.issues()[0],
            LinearRunnerInstallationIssue::CabinetDepthBelowMinimum { .. }
        ));
        assert!(matches!(
            report.issues()[1],
            LinearRunnerInstallationIssue::DrawerWidthMismatch { .. }
        ));
        assert!(matches!(
            report.issues()[2],
            LinearRunnerInstallationIssue::DrawerSideTooThick { .. }
        ));
        assert!(matches!(
            report.issues()[3],
            LinearRunnerInstallationIssue::RunnerNominalLengthMismatch {
                side: LinearRunnerSide::Right,
                ..
            }
        ));
        assert!(matches!(
            report.issues()[4],
            LinearRunnerInstallationIssue::RunnerPairLengthMismatch { .. }
        ));
        assert!(matches!(
            report.issues()[5],
            LinearRunnerInstallationIssue::RunnerPairHeightMismatch { .. }
        ));
        assert!(matches!(
            report.issues()[6],
            LinearRunnerInstallationIssue::TravelOutsideLimits { .. }
        ));
    }

    #[test]
    fn negative_travel_is_reported_as_an_actionable_limit_issue() {
        let mut installation = valid_installation();
        installation.requested_travel_mm = -1.0;
        let report = validate_linear_runner_installation(
            HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450,
            installation,
        )
        .unwrap();
        assert_eq!(
            report.issues(),
            &[LinearRunnerInstallationIssue::TravelOutsideLimits {
                actual_mm: -1.0,
                minimum_mm: 0.0,
                maximum_mm: 450.0,
            }]
        );
    }

    #[test]
    fn non_finite_or_non_positive_installation_values_fail_closed() {
        let mut installation = valid_installation();
        installation.cabinet_depth_mm = f64::NAN;
        assert_eq!(
            validate_linear_runner_installation(
                HETTICH_QUADRO_V6_PUSH_TO_OPEN_EB20_450,
                installation,
            ),
            Err(LinearRunnerInstallationError::InvalidInstallation)
        );
    }
}
