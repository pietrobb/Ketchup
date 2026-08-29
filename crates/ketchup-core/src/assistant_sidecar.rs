use crate::bottle_m6::{
    controlled_bottle_profile, finish_amount_is_conservative, inner_shell_profile,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

pub const ASSISTANT_PROTOCOL_VERSION: u16 = 2;
const MAX_ASSISTANT_MODEL_BYTES: usize = 128;
const MAX_ASSISTANT_BOXES: usize = 64;
const MAX_ASSISTANT_SUBTRACTIONS: usize = 64;
const MAX_ASSISTANT_TRANSLATIONS: usize = 100;
const MAX_ASSISTANT_PROFILE_TRANSLATIONS: usize = 1;
const MAX_ASSISTANT_ARRAYS: usize = 16;
const MAX_ASSISTANT_ARRAY_SOURCES: usize = 100;
const MAX_ASSISTANT_ARRAY_INSTANCES: u32 = 1_000;
const MAX_ASSISTANT_ARRAY_OUTPUTS: usize = 512;
const MAX_ASSISTANT_BOTTLES: usize = 8;
const MAX_ASSISTANT_TEAPOT_DIMENSION_MM: f64 = 2_000.0;
const MAX_ASSISTANT_BALLOON_TEXTS: usize = 8;
const MAX_ASSISTANT_BALLOON_TEXT_CHARS: usize = 32;
const MAX_ASSISTANT_GABLE_ROOFS: usize = 16;
const MAX_ASSISTANT_STAIRCASES: usize = 16;
const MAX_ASSISTANT_ORIENTED_BEAMS: usize = 64;
const MAX_ASSISTANT_BEAM_NOTCHES: usize = 64;
const MAX_ASSISTANT_NAME_BYTES: usize = 128;
const MAX_ASSISTANT_ABS_MM: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistantDistribution {
    PublicApi,
    PrivateOauth,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCapability {
    Chat,
    DebugObservability,
    LocalMemory,
    QueryDocument,
    ProposeWorkflowIntent,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantSubtractionIntent {
    pub size_mm: [f64; 3],
    pub origin_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBoxIntent {
    pub name: String,
    pub size_mm: [f64; 3],
    pub origin_mm: [f64; 3],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subtract_boxes: Vec<AssistantSubtractionIntent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTranslationIntent {
    pub occurrence_id: u64,
    pub delta_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantProfileTranslationIntent {
    pub definition_id: u64,
    pub body_id: u64,
    pub profile_id: u64,
    pub delta_mm: [f64; 2],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantParameterEditIntent {
    pub definition_id: u64,
    pub body_id: u64,
    pub feature_id: u64,
    pub constraint_id: Option<u64>,
    pub value_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantLinearArrayIntent {
    pub occurrence_ids: Vec<u64>,
    pub instances: u32,
    pub step_mm: [f64; 3],
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantBottleFinishKind {
    Fillet,
    Chamfer,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantTeapotIntent {
    pub handle_clearance_mm: f64,
    pub handle_tube_radius_mm: f64,
    pub spout_length_mm: f64,
    pub spout_radius_mm: f64,
    pub lid_height_mm: f64,
    pub lid_knob_radius_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantKetchupBottleIntent {
    pub body_depth_ratio: f64,
    pub cap_radius_mm: f64,
    pub cap_height_mm: f64,
    pub label_width_mm: f64,
    pub label_height_mm: f64,
    pub label_relief_mm: f64,
    pub grip_rib_count: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBottleIntent {
    pub name: String,
    pub body_radius_mm: f64,
    pub body_height_mm: f64,
    pub shoulder_rise_mm: f64,
    pub neck_radius_mm: f64,
    pub neck_height_mm: f64,
    pub wall_thickness_mm: f64,
    pub finish_kind: AssistantBottleFinishKind,
    pub finish_amount_mm: f64,
    pub origin_mm: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teapot: Option<AssistantTeapotIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ketchup_bottle: Option<AssistantKetchupBottleIntent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBalloonTextIntent {
    pub name: String,
    pub text: String,
    pub height_mm: f64,
    pub depth_mm: f64,
    pub stroke_width_mm: f64,
    pub letter_spacing_mm: f64,
    pub origin_mm: [f64; 3],
}

impl AssistantBottleIntent {
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > MAX_ASSISTANT_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err("assistant bottle name is invalid".to_owned());
        }
        if self
            .origin_mm
            .iter()
            .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
        {
            return Err("assistant bottle origin is outside the envelope".to_owned());
        }
        let dimensions = [
            self.body_radius_mm,
            self.body_height_mm,
            self.shoulder_rise_mm,
            self.neck_radius_mm,
            self.neck_height_mm,
            self.wall_thickness_mm,
            self.finish_amount_mm,
        ];
        if dimensions
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM)
            || self.neck_radius_mm >= self.body_radius_mm
        {
            return Err("assistant bottle dimensions are outside the envelope".to_owned());
        }
        let source_profile = vec![
            [0.0, 0.0],
            [self.body_radius_mm, 0.0],
            [self.body_radius_mm, self.body_height_mm],
            [
                self.neck_radius_mm,
                self.body_height_mm + self.shoulder_rise_mm,
            ],
            [
                self.neck_radius_mm,
                self.body_height_mm + self.shoulder_rise_mm + self.neck_height_mm,
            ],
            [
                0.0,
                self.body_height_mm + self.shoulder_rise_mm + self.neck_height_mm,
            ],
        ];
        let profile = controlled_bottle_profile(
            &source_profile,
            self.body_radius_mm,
            self.body_height_mm,
            self.shoulder_rise_mm,
        )
        .map_err(|_| "assistant bottle profile is unsupported".to_owned())?;
        inner_shell_profile(&profile, self.wall_thickness_mm)
            .map_err(|_| "assistant bottle wall thickness is unsupported".to_owned())?;
        if !finish_amount_is_conservative(&profile, self.finish_amount_mm) {
            return Err("assistant bottle edge finish is unsupported".to_owned());
        }
        if let Some(teapot) = &self.teapot {
            let dimensions = [
                teapot.handle_clearance_mm,
                teapot.handle_tube_radius_mm,
                teapot.spout_length_mm,
                teapot.spout_radius_mm,
                teapot.lid_height_mm,
                teapot.lid_knob_radius_mm,
            ];
            if dimensions.iter().any(|value| {
                !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_TEAPOT_DIMENSION_MM
            }) || teapot.handle_clearance_mm < teapot.handle_tube_radius_mm * 2.0
                || teapot.handle_tube_radius_mm >= self.body_radius_mm * 0.35
                || teapot.spout_length_mm < self.body_radius_mm * 0.75
                || teapot.spout_length_mm > self.body_radius_mm * 4.0
                || teapot.spout_radius_mm <= self.wall_thickness_mm
                || teapot.spout_radius_mm >= self.body_radius_mm * 0.5
                || teapot.lid_height_mm >= self.body_height_mm * 0.5
                || teapot.lid_knob_radius_mm >= self.neck_radius_mm * 0.75
            {
                return Err("assistant teapot dimensions are outside the envelope".to_owned());
            }
        }
        if self.teapot.is_some() && self.ketchup_bottle.is_some() {
            return Err("assistant bottle cannot combine vessel styles".to_owned());
        }
        if let Some(ketchup) = &self.ketchup_bottle {
            let dimensions = [
                ketchup.cap_radius_mm,
                ketchup.cap_height_mm,
                ketchup.label_width_mm,
                ketchup.label_height_mm,
                ketchup.label_relief_mm,
            ];
            if !ketchup.body_depth_ratio.is_finite()
                || !(0.5..=1.0).contains(&ketchup.body_depth_ratio)
                || dimensions.iter().any(|value| {
                    !value.is_finite()
                        || *value <= 0.0
                        || *value > MAX_ASSISTANT_TEAPOT_DIMENSION_MM
                })
                || ketchup.cap_radius_mm <= self.neck_radius_mm + self.wall_thickness_mm * 1.75
                || ketchup.cap_radius_mm >= self.body_radius_mm * 0.55
                || ketchup.cap_height_mm <= self.neck_height_mm + self.wall_thickness_mm * 2.0
                || ketchup.cap_height_mm >= self.body_height_mm * 0.35
                || ketchup.label_width_mm >= self.body_radius_mm * 1.8
                || ketchup.label_height_mm >= self.body_height_mm * 0.7
                || ketchup.label_relief_mm >= self.body_radius_mm * 0.1
                || !(8..=48).contains(&ketchup.grip_rib_count)
            {
                return Err(
                    "assistant ketchup bottle dimensions are outside the envelope".to_owned(),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantGableRoofIntent {
    pub name: String,
    pub length_mm: f64,
    pub span_mm: f64,
    pub rise_mm: f64,
    pub thickness_mm: f64,
    pub origin_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantStaircaseIntent {
    pub name: String,
    pub run_mm: f64,
    pub width_mm: f64,
    pub rise_mm: f64,
    pub step_count: u32,
    pub origin_mm: [f64; 3],
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantBeamNotchIntent {
    pub from_start_mm: f64,
    pub length_mm: f64,
    pub depth_mm: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantOrientedBeamIntent {
    pub name: String,
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    pub up_hint: [f64; 3],
    pub width_mm: f64,
    pub depth_mm: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bottom_notches: Vec<AssistantBeamNotchIntent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantModelIntent {
    pub replace_scene: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<AssistantBoxIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub translations: Vec<AssistantTranslationIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_translations: Vec<AssistantProfileTranslationIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_edits: Vec<AssistantParameterEditIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linear_arrays: Vec<AssistantLinearArrayIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bottles: Vec<AssistantBottleIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub balloon_texts: Vec<AssistantBalloonTextIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gable_roofs: Vec<AssistantGableRoofIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub staircases: Vec<AssistantStaircaseIntent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oriented_beams: Vec<AssistantOrientedBeamIntent>,
}

fn boxes_overlap(left: &AssistantSubtractionIntent, right: &AssistantSubtractionIntent) -> bool {
    (0..3).all(|axis| {
        left.origin_mm[axis] < right.origin_mm[axis] + right.size_mm[axis]
            && right.origin_mm[axis] < left.origin_mm[axis] + left.size_mm[axis]
    })
}

impl AssistantOrientedBeamIntent {
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty()
            || self.name.len() > MAX_ASSISTANT_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err("assistant oriented beam name is invalid".to_owned());
        }
        if self
            .start_mm
            .iter()
            .chain(self.end_mm.iter())
            .chain(self.up_hint.iter())
            .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            || !self.width_mm.is_finite()
            || self.width_mm <= 0.0
            || self.width_mm > MAX_ASSISTANT_ABS_MM
            || !self.depth_mm.is_finite()
            || self.depth_mm <= 0.0
            || self.depth_mm > MAX_ASSISTANT_ABS_MM
        {
            return Err("assistant oriented beam dimensions are outside the envelope".to_owned());
        }
        let axis = [
            self.end_mm[0] - self.start_mm[0],
            self.end_mm[1] - self.start_mm[1],
            self.end_mm[2] - self.start_mm[2],
        ];
        let axis_length = axis.iter().map(|value| value * value).sum::<f64>().sqrt();
        let up_length = self
            .up_hint
            .iter()
            .map(|value| value * value)
            .sum::<f64>()
            .sqrt();
        let cross = [
            axis[1] * self.up_hint[2] - axis[2] * self.up_hint[1],
            axis[2] * self.up_hint[0] - axis[0] * self.up_hint[2],
            axis[0] * self.up_hint[1] - axis[1] * self.up_hint[0],
        ];
        let cross_length = cross.iter().map(|value| value * value).sum::<f64>().sqrt();
        if axis_length <= f64::EPSILON
            || axis_length > MAX_ASSISTANT_ABS_MM
            || up_length <= f64::EPSILON
            || cross_length <= axis_length * up_length * 1.0e-6
        {
            return Err("assistant oriented beam axis or up hint is invalid".to_owned());
        }
        if self.bottom_notches.len() > MAX_ASSISTANT_BEAM_NOTCHES {
            return Err("assistant oriented beam contains too many notches".to_owned());
        }
        for notch in &self.bottom_notches {
            if !notch.from_start_mm.is_finite()
                || notch.from_start_mm < 0.0
                || !notch.length_mm.is_finite()
                || notch.length_mm <= 0.0
                || notch.from_start_mm + notch.length_mm > axis_length
                || !notch.depth_mm.is_finite()
                || notch.depth_mm <= 0.0
                || notch.depth_mm >= self.depth_mm
            {
                return Err("assistant oriented beam notch is invalid".to_owned());
            }
        }
        if self.bottom_notches.iter().enumerate().any(|(index, left)| {
            self.bottom_notches[index + 1..].iter().any(|right| {
                left.from_start_mm < right.from_start_mm + right.length_mm
                    && right.from_start_mm < left.from_start_mm + left.length_mm
            })
        }) {
            return Err("assistant oriented beam notches overlap".to_owned());
        }
        Ok(())
    }
}

impl AssistantModelIntent {
    pub fn validate(&self) -> Result<(), String> {
        if self.boxes.is_empty()
            && self.translations.is_empty()
            && self.profile_translations.is_empty()
            && self.parameter_edits.is_empty()
            && self.linear_arrays.is_empty()
            && self.bottles.is_empty()
            && self.balloon_texts.is_empty()
            && self.gable_roofs.is_empty()
            && self.staircases.is_empty()
            && self.oriented_beams.is_empty()
        {
            return Err(
                "assistant proposal must contain geometry, translations, profile translations, parameter edits, linear arrays, bottles, balloon text, roofs, staircases, or oriented beams"
                    .to_owned(),
            );
        }
        if self.boxes.len() > MAX_ASSISTANT_BOXES {
            return Err("assistant proposal contains more than 64 boxes".to_owned());
        }
        if self.translations.len() > MAX_ASSISTANT_TRANSLATIONS {
            return Err("assistant proposal contains more than 100 translations".to_owned());
        }
        if self.profile_translations.len() > MAX_ASSISTANT_PROFILE_TRANSLATIONS {
            return Err("assistant proposal contains more than one profile translation".to_owned());
        }
        if self.parameter_edits.len() > 1 {
            return Err("assistant proposal contains more than one parameter edit".to_owned());
        }
        if self.linear_arrays.len() > MAX_ASSISTANT_ARRAYS {
            return Err("assistant proposal contains too many linear arrays".to_owned());
        }
        if self.bottles.len() > MAX_ASSISTANT_BOTTLES {
            return Err("assistant proposal contains too many bottles".to_owned());
        }
        if self.balloon_texts.len() > MAX_ASSISTANT_BALLOON_TEXTS {
            return Err("assistant proposal contains too many balloon texts".to_owned());
        }
        if self.gable_roofs.len() > MAX_ASSISTANT_GABLE_ROOFS {
            return Err("assistant proposal contains too many gable roofs".to_owned());
        }
        if self.staircases.len() > MAX_ASSISTANT_STAIRCASES {
            return Err("assistant proposal contains too many staircases".to_owned());
        }
        if self.oriented_beams.len() > MAX_ASSISTANT_ORIENTED_BEAMS {
            return Err("assistant proposal contains too many oriented beams".to_owned());
        }
        for beam in &self.oriented_beams {
            beam.validate()?;
        }
        for bottle in &self.bottles {
            bottle.validate()?;
        }
        for text in &self.balloon_texts {
            let characters = text.text.chars().collect::<Vec<_>>();
            if text.name.trim().is_empty()
                || text.name.len() > MAX_ASSISTANT_NAME_BYTES
                || text.name.chars().any(char::is_control)
                || characters.is_empty()
                || characters.len() > MAX_ASSISTANT_BALLOON_TEXT_CHARS
                || characters.iter().all(|character| *character == ' ')
                || characters
                    .iter()
                    .any(|character| !matches!(character, 'A'..='Z' | '0'..='9' | ' ' | 'ˇ'))
                || !text.height_mm.is_finite()
                || !(10.0..=MAX_ASSISTANT_TEAPOT_DIMENSION_MM).contains(&text.height_mm)
                || !text.depth_mm.is_finite()
                || !(text.height_mm * 0.1..=text.height_mm * 0.8).contains(&text.depth_mm)
                || !text.stroke_width_mm.is_finite()
                || !(text.height_mm * 0.08..=text.height_mm * 0.24).contains(&text.stroke_width_mm)
                || !text.letter_spacing_mm.is_finite()
                || !(0.0..=text.height_mm).contains(&text.letter_spacing_mm)
                || text
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant balloon text is invalid".to_owned());
            }
        }
        for roof in &self.gable_roofs {
            if roof.name.trim().is_empty()
                || roof.name.len() > MAX_ASSISTANT_NAME_BYTES
                || roof.name.chars().any(char::is_control)
                || [
                    roof.length_mm,
                    roof.span_mm,
                    roof.rise_mm,
                    roof.thickness_mm,
                ]
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM)
                || roof.thickness_mm >= roof.rise_mm
                || roof
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant gable roof is invalid".to_owned());
            }
        }
        for stairs in &self.staircases {
            let tread_mm = stairs.run_mm / f64::from(stairs.step_count.max(1));
            let riser_mm = stairs.rise_mm / f64::from(stairs.step_count.max(1));
            if stairs.name.trim().is_empty()
                || stairs.name.len() > MAX_ASSISTANT_NAME_BYTES
                || stairs.name.chars().any(char::is_control)
                || [stairs.run_mm, stairs.width_mm, stairs.rise_mm]
                    .iter()
                    .any(|value| {
                        !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM
                    })
                || !(2..=64).contains(&stairs.step_count)
                || !(150.0..=450.0).contains(&tread_mm)
                || !(100.0..=250.0).contains(&riser_mm)
                || stairs.width_mm < 500.0
                || stairs
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant staircase is invalid".to_owned());
            }
        }
        if self.replace_scene
            && (!self.translations.is_empty()
                || !self.profile_translations.is_empty()
                || !self.parameter_edits.is_empty()
                || !self.linear_arrays.is_empty())
        {
            return Err("assistant edits of existing geometry cannot replace the scene".to_owned());
        }
        if !self.profile_translations.is_empty()
            && (!self.boxes.is_empty()
                || !self.translations.is_empty()
                || !self.parameter_edits.is_empty()
                || !self.linear_arrays.is_empty()
                || !self.bottles.is_empty()
                || !self.balloon_texts.is_empty()
                || !self.gable_roofs.is_empty()
                || !self.staircases.is_empty()
                || !self.oriented_beams.is_empty())
        {
            return Err("assistant profile translation cannot mix geometry mutations".to_owned());
        }
        if !self.parameter_edits.is_empty()
            && (!self.boxes.is_empty()
                || !self.translations.is_empty()
                || !self.profile_translations.is_empty()
                || !self.linear_arrays.is_empty()
                || !self.bottles.is_empty()
                || !self.balloon_texts.is_empty()
                || !self.gable_roofs.is_empty()
                || !self.staircases.is_empty()
                || !self.oriented_beams.is_empty())
        {
            return Err("assistant parameter edit cannot mix geometry mutations".to_owned());
        }
        let mut translated_occurrences = BTreeSet::new();
        for translation in &self.translations {
            if translation.occurrence_id == 0
                || !translated_occurrences.insert(translation.occurrence_id)
                || translation
                    .delta_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err("assistant translation is invalid".to_owned());
            }
        }
        for translation in &self.profile_translations {
            if translation.definition_id == 0
                || translation.body_id == 0
                || translation.profile_id == 0
                || translation
                    .delta_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
                || translation.delta_mm.iter().all(|value| *value == 0.0)
            {
                return Err("assistant profile translation is invalid".to_owned());
            }
        }
        for edit in &self.parameter_edits {
            if edit.definition_id == 0
                || edit.body_id == 0
                || edit.feature_id == 0
                || edit.constraint_id == Some(0)
                || !edit.value_mm.is_finite()
                || edit.value_mm <= 0.0
                || edit.value_mm > MAX_ASSISTANT_ABS_MM
            {
                return Err("assistant parameter edit is invalid".to_owned());
            }
        }
        let mut array_outputs = 0usize;
        for array in &self.linear_arrays {
            let mut occurrence_ids = BTreeSet::new();
            if array.occurrence_ids.is_empty()
                || array.occurrence_ids.len() > MAX_ASSISTANT_ARRAY_SOURCES
                || array.instances < 2
                || array.instances > MAX_ASSISTANT_ARRAY_INSTANCES
                || array
                    .occurrence_ids
                    .iter()
                    .any(|id| *id == 0 || !occurrence_ids.insert(*id))
                || array
                    .step_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
                || array.step_mm.iter().all(|value| *value == 0.0)
                || array.step_mm.iter().any(|value| {
                    (*value * f64::from(array.instances.saturating_sub(1))).abs()
                        > MAX_ASSISTANT_ABS_MM
                })
            {
                return Err("assistant linear array is invalid".to_owned());
            }
            let Some(outputs) = array
                .occurrence_ids
                .len()
                .checked_mul(array.instances.saturating_sub(1) as usize)
            else {
                return Err("assistant linear array output count is invalid".to_owned());
            };
            let Some(total_outputs) = array_outputs.checked_add(outputs) else {
                return Err("assistant linear array output count is invalid".to_owned());
            };
            if total_outputs > MAX_ASSISTANT_ARRAY_OUTPUTS {
                return Err("assistant proposal creates too many array occurrences".to_owned());
            }
            array_outputs = total_outputs;
        }
        for item in &self.boxes {
            if item.name.trim().is_empty()
                || item.name.len() > MAX_ASSISTANT_NAME_BYTES
                || item.name.chars().any(char::is_control)
            {
                return Err("assistant box name is invalid".to_owned());
            }
            if item
                .size_mm
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0 || *value > MAX_ASSISTANT_ABS_MM)
                || item
                    .origin_mm
                    .iter()
                    .any(|value| !value.is_finite() || value.abs() > MAX_ASSISTANT_ABS_MM)
            {
                return Err(
                    "assistant box dimensions or origin are outside the envelope".to_owned(),
                );
            }
            if item.subtract_boxes.len() > MAX_ASSISTANT_SUBTRACTIONS {
                return Err("assistant body contains more than 64 subtractions".to_owned());
            }
            let [width, depth, height] = item.size_mm;
            for subtraction in &item.subtract_boxes {
                let [cut_width, cut_depth, cut_height] = subtraction.size_mm;
                let [cut_x, cut_y, cut_z] = subtraction.origin_mm;
                let retained_through_opening = cut_z == 0.0
                    && cut_height == height
                    && cut_x > 0.0
                    && cut_y > 0.0
                    && cut_x + cut_width < width
                    && cut_y + cut_depth < depth;
                if subtraction
                    .size_mm
                    .iter()
                    .any(|value| !value.is_finite() || *value <= 0.0)
                    || subtraction.origin_mm.iter().any(|value| !value.is_finite())
                    || cut_x < 0.0
                    || cut_y < 0.0
                    || cut_z < 0.0
                    || cut_x + cut_width > width
                    || cut_y + cut_depth > depth
                    || cut_z + cut_height > height
                    || (cut_height >= height && !retained_through_opening)
                {
                    return Err("assistant subtraction is outside its body".to_owned());
                }
            }
            if item.subtract_boxes.iter().enumerate().any(|(index, left)| {
                item.subtract_boxes[index + 1..]
                    .iter()
                    .any(|right| boxes_overlap(left, right))
            }) {
                return Err("assistant subtractions overlap".to_owned());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantApiDiagnostics {
    pub provider: String,
    pub model: String,
    pub duration_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub stop_reason: String,
    pub system_prompt: String,
    pub request_payload: serde_json::Value,
    pub response_text: String,
}

impl AssistantApiDiagnostics {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider.is_empty()
            || self.provider.len() > MAX_ASSISTANT_MODEL_BYTES
            || self.model.is_empty()
            || self.model.len() > MAX_ASSISTANT_MODEL_BYTES
            || self.system_prompt.len() > 64 * 1024
            || self.response_text.len() > 64 * 1024
            || serde_json::to_vec(&self.request_payload)
                .map_or(true, |bytes| bytes.len() > 128 * 1024)
        {
            return Err("assistant API diagnostics exceed their bounded envelope".to_owned());
        }
        Ok(())
    }

    #[must_use]
    pub const fn total_tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantChatResult {
    pub message: String,
    pub model_intent: Option<AssistantModelIntent>,
}

impl AssistantChatResult {
    pub fn validate(&self) -> Result<(), String> {
        if self.message.trim().is_empty() {
            return Err("assistant returned an empty message".to_owned());
        }
        if let Some(intent) = &self.model_intent {
            intent.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantHandshake {
    pub protocol_version: u16,
    pub distribution: AssistantDistribution,
    pub provider: String,
    pub model: String,
    pub capabilities: BTreeSet<AssistantCapability>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssistantHandshakeError {
    InvalidJson(String),
    UnsupportedProtocolVersion(u16),
    UnsupportedDistribution(AssistantDistribution),
    UnsupportedProvider(String),
    UnsupportedModel(String),
    UnsupportedCapabilities(BTreeSet<AssistantCapability>),
}

impl fmt::Display for AssistantHandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(error) => write!(formatter, "invalid assistant handshake: {error}"),
            Self::UnsupportedProtocolVersion(version) => {
                write!(
                    formatter,
                    "unsupported assistant protocol version: {version}"
                )
            }
            Self::UnsupportedDistribution(distribution) => {
                write!(
                    formatter,
                    "unsupported assistant distribution: {distribution:?}"
                )
            }
            Self::UnsupportedProvider(provider) => {
                write!(formatter, "unsupported assistant provider: {provider}")
            }
            Self::UnsupportedModel(model) => {
                write!(formatter, "unsupported assistant model: {model}")
            }
            Self::UnsupportedCapabilities(capabilities) => {
                write!(
                    formatter,
                    "unsupported assistant capabilities: {capabilities:?}"
                )
            }
        }
    }
}

impl std::error::Error for AssistantHandshakeError {}

impl AssistantHandshake {
    pub fn parse_and_validate(line: &str) -> Result<Self, AssistantHandshakeError> {
        let handshake: Self = serde_json::from_str(line)
            .map_err(|error| AssistantHandshakeError::InvalidJson(error.to_string()))?;
        handshake.validate()?;
        Ok(handshake)
    }

    pub fn validate(&self) -> Result<(), AssistantHandshakeError> {
        if self.protocol_version != ASSISTANT_PROTOCOL_VERSION {
            return Err(AssistantHandshakeError::UnsupportedProtocolVersion(
                self.protocol_version,
            ));
        }

        if !distribution_is_enabled(self.distribution) {
            return Err(AssistantHandshakeError::UnsupportedDistribution(
                self.distribution,
            ));
        }

        let provider_supported = match self.distribution {
            AssistantDistribution::PublicApi => {
                matches!(self.provider.as_str(), "anthropic-api" | "openai-api")
            }
            AssistantDistribution::PrivateOauth => {
                matches!(self.provider.as_str(), "claude-code-oauth" | "codex-oauth")
            }
        };
        if !provider_supported {
            return Err(AssistantHandshakeError::UnsupportedProvider(
                self.provider.clone(),
            ));
        }
        if self.model.is_empty()
            || self.model.len() > MAX_ASSISTANT_MODEL_BYTES
            || !self.model.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
        {
            return Err(AssistantHandshakeError::UnsupportedModel(
                self.model.clone(),
            ));
        }

        let allowed = BTreeSet::from([
            AssistantCapability::Chat,
            AssistantCapability::DebugObservability,
            AssistantCapability::LocalMemory,
            AssistantCapability::QueryDocument,
            AssistantCapability::ProposeWorkflowIntent,
        ]);
        if !self.capabilities.is_subset(&allowed) {
            return Err(AssistantHandshakeError::UnsupportedCapabilities(
                self.capabilities.difference(&allowed).copied().collect(),
            ));
        }

        Ok(())
    }
}

#[must_use]
pub const fn distribution_is_enabled(distribution: AssistantDistribution) -> bool {
    match distribution {
        AssistantDistribution::PublicApi => true,
        AssistantDistribution::PrivateOauth => cfg!(feature = "private-oauth"),
    }
}
