use super::*;
use ketchup_tolerance::{THREAD_APPROXIMATION_MM, limits};

/// The groove of a modelled thread; see [`ExactBackend::thread_groove`].
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadGrooveSpec {
    /// A point on the axis where the groove's helix crosses `reference`: the
    /// thread's phase.
    pub origin_mm: [f64; 3],
    /// Unit direction of the axis.
    pub axis: [f64; 3],
    /// Unit direction square to `axis`.
    pub reference: [f64; 3],
    pub pitch_mm: f64,
    /// Where the helix starts and ends along the axis from `origin_mm`.
    pub start_mm: f64,
    pub end_mm: f64,
    pub left_handed: bool,
    /// The closed groove section in the axial half-plane: radius from the
    /// axis and offset along it from the helix, in order around the section.
    pub profile_mm: Vec<[f64; 2]>,
}

impl ThreadGrooveSpec {
    /// The request digest and the native section of a valid spec.
    fn checked(
        &self,
        operation: &'static str,
    ) -> Result<(String, Vec<ffi::NativePoint>), GeometryError> {
        let mut values = Vec::with_capacity(13 + 2 * self.profile_mm.len());
        values.extend(self.origin_mm);
        values.extend(self.axis);
        values.extend(self.reference);
        values.extend([self.pitch_mm, self.start_mm, self.end_mm]);
        values.push(if self.left_handed { 1.0 } else { 0.0 });
        values.extend(self.profile_mm.iter().flatten());
        let input = format!("{operation}:{:?}", digest_bits(&values));
        for (index, value) in values.iter().enumerate() {
            validate_coordinate(*value, &format!("value_{index}"), operation, &input)?;
        }
        let unit = |vector: [f64; 3]| (spatial_length(vector) - 1.0).abs() <= ROUNDING;
        let square = (self.axis[0] * self.reference[0]
            + self.axis[1] * self.reference[1]
            + self.axis[2] * self.reference[2])
            .abs()
            <= ROUNDING;
        let (min_offset, max_offset) = self
            .profile_mm
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), point| {
                (low.min(point[1]), high.max(point[1]))
            });
        if !unit(self.axis)
            || !unit(self.reference)
            || !square
            || !(3..=limits::PATH_SEGMENTS).contains(&self.profile_mm.len())
            || self.profile_mm.iter().any(|point| point[0] <= 0.0)
            || max_offset - min_offset >= self.pitch_mm
            || self.end_mm <= self.start_mm
        {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "A thread groove needs square unit axis and reference, a section of 3 to 64 \
                 points off the axis narrower than the pitch, and an increasing range"
                    .to_owned(),
            ));
        }
        validate_length(self.pitch_mm, "pitch_mm", operation, &input)?;
        let profile = self
            .profile_mm
            .iter()
            .map(|point| native_spatial_point([point[0], point[1], 0.0]))
            .collect();
        Ok((input, profile))
    }

    /// Origin, axis, reference, pitch, start and end, as native code reads them.
    fn helix(&self) -> Vec<f64> {
        let mut helix = [self.origin_mm, self.axis, self.reference].concat();
        helix.extend([self.pitch_mm, self.start_mm, self.end_mm]);
        helix
    }
}

impl ExactBackend {
    /// The solid a thread removes: `profile_mm` carried by the screw motion
    /// about the axis, `pitch_mm` per turn, from `start_mm` to `end_mm`. At
    /// axial position s the section lies at angle 2π·s/pitch from
    /// `reference` (clockwise for a left-handed thread), so two parts with
    /// the same origin and reference share the thread's phase.
    pub fn thread_groove(&self, spec: &ThreadGrooveSpec) -> Result<ExactOpOutput, GeometryError> {
        let operation = "thread_groove";
        let (input, profile) = spec.checked(operation)?;
        collect_output(
            ffi::thread_groove_native(
                &spec.helix(),
                spec.left_handed,
                &profile,
                THREAD_APPROXIMATION_MM,
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }

    /// `target` with the groove of [`Self::thread_groove`] cut out of it: a
    /// modelled thread. Its faces carry the semantic role
    /// `thread.side.<index of the section edge>`.
    pub fn cut_thread(
        &self,
        target: &ExactBody,
        spec: &ThreadGrooveSpec,
    ) -> Result<ExactOpOutput, GeometryError> {
        let operation = "cut_thread";
        let (input, profile) = spec.checked(operation)?;
        let input = format!("{input}:{}", target.result_fingerprint);
        let native_target = target.owned_native("Target exact body", operation, &input)?;
        collect_output(
            ffi::thread_cut_native(
                native_target,
                &spec.helix(),
                spec.left_handed,
                &profile,
                THREAD_APPROXIMATION_MM,
            ),
            operation,
            &input,
            HistoryConfidence::Partial,
        )
    }
}
