use crate::assembly_joint::AssemblyJointId;

pub const ASSEMBLY_MOTION_COUPLING_SCHEMA_V1: &str = "ketchup.assembly-motion-coupling.v1";

const MAX_TOOTH_COUNT: u32 = 1_000_000;
const MAX_PITCH_DIMENSION_MM: f64 = 1_000_000.0;
const MAX_REFERENCE_POSITION: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AssemblyMotionCouplingId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyMotionDirection {
    Same,
    Opposite,
}

impl AssemblyMotionDirection {
    #[must_use]
    pub const fn sign(self) -> f64 {
        match self {
            Self::Same => 1.0,
            Self::Opposite => -1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GearMeshKind {
    External,
    Internal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrewHandedness {
    Right,
    Left,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CoupledJointKind {
    Revolute,
    Prismatic,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AssemblyTransmissionKind {
    GearPair {
        input_teeth: u32,
        output_teeth: u32,
        mesh: GearMeshKind,
    },
    Belt {
        input_pitch_diameter_mm: f64,
        output_pitch_diameter_mm: f64,
        crossed: bool,
    },
    Chain {
        input_sprocket_teeth: u32,
        output_sprocket_teeth: u32,
    },
    RackAndPinion {
        pinion_pitch_diameter_mm: f64,
        direction: AssemblyMotionDirection,
    },
    LeadScrew {
        lead_mm_per_revolution: f64,
        handedness: ScrewHandedness,
    },
}

impl AssemblyTransmissionKind {
    #[must_use]
    pub const fn joint_kinds(self) -> (CoupledJointKind, CoupledJointKind) {
        match self {
            Self::GearPair { .. } | Self::Belt { .. } | Self::Chain { .. } => {
                (CoupledJointKind::Revolute, CoupledJointKind::Revolute)
            }
            Self::RackAndPinion { .. } | Self::LeadScrew { .. } => {
                (CoupledJointKind::Revolute, CoupledJointKind::Prismatic)
            }
        }
    }

    #[must_use]
    pub fn scale(self) -> f64 {
        match self {
            Self::GearPair {
                input_teeth,
                output_teeth,
                mesh,
            } => {
                let direction = match mesh {
                    GearMeshKind::External => -1.0,
                    GearMeshKind::Internal => 1.0,
                };
                direction * f64::from(input_teeth) / f64::from(output_teeth)
            }
            Self::Belt {
                input_pitch_diameter_mm,
                output_pitch_diameter_mm,
                crossed,
            } => {
                let direction = if crossed { -1.0 } else { 1.0 };
                direction * input_pitch_diameter_mm / output_pitch_diameter_mm
            }
            Self::Chain {
                input_sprocket_teeth,
                output_sprocket_teeth,
            } => f64::from(input_sprocket_teeth) / f64::from(output_sprocket_teeth),
            Self::RackAndPinion {
                pinion_pitch_diameter_mm,
                direction,
            } => direction.sign() * std::f64::consts::PI * pinion_pitch_diameter_mm / 360.0,
            Self::LeadScrew {
                lead_mm_per_revolution,
                handedness,
            } => {
                let direction = match handedness {
                    ScrewHandedness::Right => 1.0,
                    ScrewHandedness::Left => -1.0,
                };
                direction * lead_mm_per_revolution / 360.0
            }
        }
    }

    #[must_use]
    pub fn is_valid(self) -> bool {
        match self {
            Self::GearPair {
                input_teeth,
                output_teeth,
                ..
            }
            | Self::Chain {
                input_sprocket_teeth: input_teeth,
                output_sprocket_teeth: output_teeth,
            } => valid_count(input_teeth) && valid_count(output_teeth),
            Self::Belt {
                input_pitch_diameter_mm,
                output_pitch_diameter_mm,
                ..
            } => {
                valid_pitch_dimension(input_pitch_diameter_mm)
                    && valid_pitch_dimension(output_pitch_diameter_mm)
            }
            Self::RackAndPinion {
                pinion_pitch_diameter_mm,
                ..
            } => valid_pitch_dimension(pinion_pitch_diameter_mm),
            Self::LeadScrew {
                lead_mm_per_revolution,
                ..
            } => valid_pitch_dimension(lead_mm_per_revolution),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::GearPair { .. } => "gear_pair",
            Self::Belt { .. } => "belt",
            Self::Chain { .. } => "chain",
            Self::RackAndPinion { .. } => "rack_and_pinion",
            Self::LeadScrew { .. } => "lead_screw",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssemblyMotionCoupling {
    pub(crate) schema: String,
    pub(crate) id: AssemblyMotionCouplingId,
    pub(crate) input_joint_id: AssemblyJointId,
    pub(crate) output_joint_id: AssemblyJointId,
    pub(crate) input_reference_position: f64,
    pub(crate) output_reference_position: f64,
    pub(crate) transmission: AssemblyTransmissionKind,
}

impl AssemblyMotionCoupling {
    #[must_use]
    pub fn new(
        id: AssemblyMotionCouplingId,
        input_joint_id: AssemblyJointId,
        output_joint_id: AssemblyJointId,
        input_reference_position: f64,
        output_reference_position: f64,
        transmission: AssemblyTransmissionKind,
    ) -> Self {
        Self {
            schema: ASSEMBLY_MOTION_COUPLING_SCHEMA_V1.to_owned(),
            id,
            input_joint_id,
            output_joint_id,
            input_reference_position,
            output_reference_position,
            transmission,
        }
    }

    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[must_use]
    pub const fn id(&self) -> AssemblyMotionCouplingId {
        self.id
    }

    #[must_use]
    pub const fn input_joint_id(&self) -> AssemblyJointId {
        self.input_joint_id
    }

    #[must_use]
    pub const fn output_joint_id(&self) -> AssemblyJointId {
        self.output_joint_id
    }

    #[must_use]
    pub const fn input_reference_position(&self) -> f64 {
        self.input_reference_position
    }

    #[must_use]
    pub const fn output_reference_position(&self) -> f64 {
        self.output_reference_position
    }

    #[must_use]
    pub const fn transmission(&self) -> AssemblyTransmissionKind {
        self.transmission
    }

    #[must_use]
    pub fn output_position(&self, input_position: f64) -> f64 {
        self.output_reference_position
            + self.transmission.scale() * (input_position - self.input_reference_position)
    }

    #[must_use]
    pub fn input_position(&self, output_position: f64) -> f64 {
        self.input_reference_position
            + (output_position - self.output_reference_position) / self.transmission.scale()
    }

    fn has_finite_affine_transform(&self) -> bool {
        let scale = self.transmission.scale();
        let offset = self.output_reference_position - scale * self.input_reference_position;
        scale.is_finite()
            && scale != 0.0
            && scale.recip().is_finite()
            && offset.is_finite()
            && (-offset / scale).is_finite()
    }

    #[must_use]
    pub fn has_valid_shape(&self) -> bool {
        self.schema == ASSEMBLY_MOTION_COUPLING_SCHEMA_V1
            && self.id.0 != 0
            && self.input_joint_id.0 != 0
            && self.output_joint_id.0 != 0
            && self.input_joint_id != self.output_joint_id
            && valid_reference(self.input_reference_position)
            && valid_reference(self.output_reference_position)
            && self.transmission.is_valid()
            && self.has_finite_affine_transform()
    }
}

fn valid_count(value: u32) -> bool {
    (1..=MAX_TOOTH_COUNT).contains(&value)
}

fn valid_pitch_dimension(value: f64) -> bool {
    value.is_finite() && value > 0.0 && value <= MAX_PITCH_DIMENSION_MM
}

fn valid_reference(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_REFERENCE_POSITION
}
