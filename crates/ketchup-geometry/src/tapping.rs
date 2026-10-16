//! How an ISO metric coarse thread is tapped into a drilled hole: one rule
//! for where a tapped hole is made and for where it is exported to a machine.
//! The table holds the same values, from the same sources, as the program
//! prelude's `METRIC_COARSE_PITCH`, `METRIC_TAP_DRILL` and
//! `METRIC_BLIND_HOLE_RUNOUT`; a ketchup-program test keeps the two equal.

use std::fmt;

/// The coarse pitch P of an ISO metric thread, its tap drill and how much
/// deeper than the thread a blind hole is drilled, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricTapping {
    pub pitch_mm: f64,
    pub tap_drill_mm: f64,
    pub blind_hole_runout_mm: f64,
}

/// How far a tapped hole may sit from its tap drill: half a 0.1 mm drill step,
/// so a hole drilled D − P (M8: 6.75 for the 6.8 drill) still takes the thread.
pub const TAP_DRILL_STEP_TOLERANCE_MM: f64 = 0.05;

const fn tapping(pitch_mm: f64, tap_drill_mm: f64, blind_hole_runout_mm: f64) -> MetricTapping {
    MetricTapping {
        pitch_mm,
        tap_drill_mm,
        blind_hole_runout_mm,
    }
}

/// Size → tapping of the ISO metric coarse threads. Tap drills: D − P except
/// M8 (6.8) and M12 (10.2), as drills come in 0.1 mm steps. Runouts: DIN
/// 76-1:2004 table 2, e1 regular case.
pub const METRIC_COARSE_TAPPING: [(&str, MetricTapping); 13] = [
    ("M3", tapping(0.5, 2.5, 2.8)),
    ("M4", tapping(0.7, 3.3, 3.8)),
    ("M5", tapping(0.8, 4.2, 4.2)),
    ("M6", tapping(1.0, 5.0, 5.1)),
    ("M8", tapping(1.25, 6.8, 6.2)),
    ("M10", tapping(1.5, 8.5, 7.3)),
    ("M12", tapping(1.75, 10.2, 8.3)),
    ("M14", tapping(2.0, 12.0, 9.3)),
    ("M16", tapping(2.0, 14.0, 9.3)),
    ("M18", tapping(2.5, 15.5, 11.2)),
    ("M20", tapping(2.5, 17.5, 11.2)),
    ("M22", tapping(2.5, 19.5, 11.2)),
    ("M24", tapping(3.0, 21.0, 13.1)),
];

/// The tapping of the coarse thread `size` ("M8"), `None` for another size.
#[must_use]
pub fn metric_coarse_tapping(size: &str) -> Option<MetricTapping> {
    METRIC_COARSE_TAPPING
        .iter()
        .find(|(name, _)| *name == size)
        .map(|(_, tapping)| *tapping)
}

/// Why a hole does not take the thread recorded on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TappedHoleMismatch {
    /// The size is not an ISO metric coarse thread of the table.
    UnknownSize,
    /// The recorded pitch is not the size's coarse pitch.
    Pitch { coarse_pitch_mm: f64 },
    /// The hole is not the size's tap drill.
    Drill {
        tap_drill_mm: f64,
        hole_diameter_mm: f64,
    },
    /// The thread is deeper than the hole takes: all of a through hole, a
    /// blind one less the tap's runout.
    TooDeep {
        deepest_thread_mm: f64,
        hole_depth_mm: f64,
    },
}

// The table and the drilled holes give finite lengths only.
impl Eq for TappedHoleMismatch {}

impl fmt::Display for TappedHoleMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSize => formatter.write_str("the size is not an ISO metric coarse thread"),
            Self::Pitch { coarse_pitch_mm } => {
                write!(formatter, "the size's coarse pitch is {coarse_pitch_mm} mm")
            }
            Self::Drill {
                tap_drill_mm,
                hole_diameter_mm,
            } => write!(
                formatter,
                "it is tapped into its {tap_drill_mm} mm tap drill hole; this hole is \
                 {hole_diameter_mm} mm"
            ),
            Self::TooDeep {
                deepest_thread_mm,
                hole_depth_mm,
            } => write!(
                formatter,
                "a {hole_depth_mm} mm hole takes a thread at most {deepest_thread_mm} mm deep"
            ),
        }
    }
}

impl MetricTapping {
    /// Whether a `diameter_mm` hole takes this thread: its tap drill or one
    /// within half a drill step of it, within the model `tolerance_mm`.
    #[must_use]
    pub fn fits_drill(&self, diameter_mm: f64, tolerance_mm: f64) -> bool {
        (diameter_mm - self.tap_drill_mm).abs() <= TAP_DRILL_STEP_TOLERANCE_MM + tolerance_mm
    }

    /// The deepest thread a hole `hole_depth_mm` deep takes: all of a
    /// `through` hole, a blind one less the runout the tap needs below it.
    #[must_use]
    pub fn deepest_thread_mm(&self, hole_depth_mm: f64, through: bool) -> f64 {
        if through {
            hole_depth_mm
        } else {
            hole_depth_mm - self.blind_hole_runout_mm
        }
    }

    /// The shallowest blind hole a thread `thread_depth_mm` deep takes, rounded
    /// up to the µm: rounded down it would be too shallow for [`TappedHole::check`].
    /// The nanometre slack keeps a depth already whole in µm from gaining one.
    #[must_use]
    pub fn blind_hole_depth_mm(&self, thread_depth_mm: f64) -> f64 {
        ((thread_depth_mm + self.blind_hole_runout_mm) * 1000.0 - 1e-6).ceil() / 1000.0
    }
}

/// A hole as drilled and the thread recorded on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TappedHole<'a> {
    pub size: &'a str,
    pub pitch_mm: f64,
    pub thread_depth_mm: f64,
    pub hole_diameter_mm: f64,
    pub hole_depth_mm: f64,
    pub through: bool,
}

impl TappedHole<'_> {
    /// Whether the hole takes its thread, within the model `tolerance_mm`:
    /// the size is in the table with this pitch, the hole is its tap drill
    /// and at least as deep as the thread needs.
    ///
    /// # Errors
    /// The first [`TappedHoleMismatch`] the hole has.
    pub fn check(&self, tolerance_mm: f64) -> Result<(), TappedHoleMismatch> {
        let tapping = metric_coarse_tapping(self.size).ok_or(TappedHoleMismatch::UnknownSize)?;
        if (self.pitch_mm - tapping.pitch_mm).abs() > tolerance_mm {
            return Err(TappedHoleMismatch::Pitch {
                coarse_pitch_mm: tapping.pitch_mm,
            });
        }
        if !tapping.fits_drill(self.hole_diameter_mm, tolerance_mm) {
            return Err(TappedHoleMismatch::Drill {
                tap_drill_mm: tapping.tap_drill_mm,
                hole_diameter_mm: self.hole_diameter_mm,
            });
        }
        let deepest_thread_mm = tapping.deepest_thread_mm(self.hole_depth_mm, self.through);
        if self.thread_depth_mm > deepest_thread_mm + tolerance_mm {
            return Err(TappedHoleMismatch::TooDeep {
                deepest_thread_mm,
                hole_depth_mm: self.hole_depth_mm,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m8(thread_depth_mm: f64, hole_diameter_mm: f64, hole_depth_mm: f64) -> TappedHole<'static> {
        TappedHole {
            size: "M8",
            pitch_mm: 1.25,
            thread_depth_mm,
            hole_diameter_mm,
            hole_depth_mm,
            through: false,
        }
    }

    #[test]
    fn a_blind_hole_takes_its_thread_only_with_the_runout_below_it() {
        assert_eq!(m8(12.0, 6.8, 18.2).check(1e-6), Ok(()));
        assert_eq!(m8(12.0, 6.75, 18.2).check(1e-6), Ok(()));
        assert_eq!(
            m8(12.0, 6.8, 12.0).check(1e-6),
            Err(TappedHoleMismatch::TooDeep {
                deepest_thread_mm: 12.0 - 6.2,
                hole_depth_mm: 12.0
            })
        );
        let through = TappedHole {
            through: true,
            ..m8(12.0, 6.8, 12.0)
        };
        assert_eq!(through.check(1e-6), Ok(()));
    }

    #[test]
    fn the_hole_sized_for_a_thread_takes_it_at_the_model_tolerance() {
        let tapping = metric_coarse_tapping("M8").unwrap();
        for thread_depth_mm in [10.0 / 3.0, 2.0 / 3.0, 12.0, 0.0004, 5.0 + 1e-9] {
            let hole_depth_mm = tapping.blind_hole_depth_mm(thread_depth_mm);
            assert_eq!(
                m8(thread_depth_mm, 6.8, hole_depth_mm).check(1e-7),
                Ok(()),
                "{thread_depth_mm} mm thread in a {hole_depth_mm} mm hole"
            );
            assert!(
                hole_depth_mm < thread_depth_mm + tapping.blind_hole_runout_mm + 0.001,
                "{hole_depth_mm} is more than a µm deeper than needed"
            );
        }
        assert_eq!(tapping.blind_hole_depth_mm(12.0), 18.2);
    }

    #[test]
    fn a_hole_of_another_drill_size_or_pitch_or_size_is_named() {
        assert!(matches!(
            m8(10.0, 8.0, 30.0).check(1e-6),
            Err(TappedHoleMismatch::Drill { tap_drill_mm, .. }) if tap_drill_mm == 6.8
        ));
        let fine = TappedHole {
            pitch_mm: 1.0,
            ..m8(10.0, 6.8, 30.0)
        };
        assert!(matches!(
            fine.check(1e-6),
            Err(TappedHoleMismatch::Pitch { .. })
        ));
        let unknown = TappedHole {
            size: "M7",
            ..m8(10.0, 6.8, 30.0)
        };
        assert_eq!(unknown.check(1e-6), Err(TappedHoleMismatch::UnknownSize));
    }
}
