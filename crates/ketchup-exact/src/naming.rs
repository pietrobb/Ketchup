//! Program-stable names for faces and edges (prototype).
//!
//! A rule program has to point at a face or an edge before anything is
//! evaluated, and the reference has to survive a changed number. Names here
//! come only from the program, never from OCCT ordinals or fingerprints:
//! - a swept face is named after the profile segment that swept it;
//! - the caps of an extrusion or a partial revolve are `start` and `end`;
//! - a face made by a finish is named after the faces around what it
//!   replaced: `fillet(a,b)` for an edge, `fillet(a,b,c)` for a corner blend.
//!   OCCT finishes whole tangent chains, so this also names edges the
//!   program never selected;
//! - a face left by a cutting tool is `tool.face`;
//! - a face an operation splits becomes `name#1`, `name#2`, ... in geometric
//!   order (x, then y, then z of the centroid).
//!
//! OCCT history carries the names through every operation. An edge is the
//! pair of faces it separates. A name that no longer exists is an error that
//! says which one; nothing is ever resolved by guessing.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{
    EdgeFinish, ExactBackend, ExactOpOutput, GeometryError, GeometryErrorCode, HistoryConfidence,
    PlanarProfileSegment, collect_output, ffi, flatten_planar_segments, planar_segment_endpoints,
    planar_segment_signed_area, reverse_planar_segments, validate_general_revolve_profile,
    validate_length, validate_mixed_profile,
};

/// One profile segment and the name its swept face will carry.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedSegment {
    pub name: String,
    pub segment: PlanarProfileSegment,
}

impl NamedSegment {
    #[must_use]
    pub fn new(name: impl Into<String>, segment: PlanarProfileSegment) -> Self {
        Self {
            name: name.into(),
            segment,
        }
    }
}

/// An exact solid whose every face has a program name.
pub struct NamedBody {
    pub output: ExactOpOutput,
    names: Vec<String>,
}

impl fmt::Debug for NamedBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NamedBody")
            .field("names", &self.names)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NamingError {
    Geometry(GeometryError),
    InvalidName(String),
    UnknownFace {
        name: String,
        available: Vec<String>,
    },
    UnknownEdge {
        first: String,
        second: String,
        neighbours_of_first: Vec<String>,
    },
    AmbiguousEdge {
        first: String,
        second: String,
        count: usize,
    },
    UnnamedFaces {
        operation: &'static str,
        ordinals: Vec<u32>,
    },
}

impl fmt::Display for NamingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Geometry(error) => write!(formatter, "{error}"),
            Self::InvalidName(name) => write!(
                formatter,
                "'{name}' is not a valid face name (empty, reserved, duplicate, or contains one of # , ( ) . :)"
            ),
            Self::UnknownFace { name, available } => write!(
                formatter,
                "face '{name}' does not exist; faces are: {}",
                available.join(", ")
            ),
            Self::UnknownEdge {
                first,
                second,
                neighbours_of_first,
            } => write!(
                formatter,
                "faces '{first}' and '{second}' do not share an edge; '{first}' borders: {}",
                neighbours_of_first.join(", ")
            ),
            Self::AmbiguousEdge {
                first,
                second,
                count,
            } => write!(
                formatter,
                "faces '{first}' and '{second}' share {count} edges; name one of the split faces"
            ),
            Self::UnnamedFaces {
                operation,
                ordinals,
            } => write!(
                formatter,
                "{operation} produced {} face(s) with no program origin",
                ordinals.len()
            ),
        }
    }
}

impl std::error::Error for NamingError {}

impl From<GeometryError> for NamingError {
    fn from(error: GeometryError) -> Self {
        Self::Geometry(error)
    }
}

impl NamedBody {
    #[must_use]
    pub fn face_names(&self) -> &[String] {
        &self.names
    }

    #[must_use]
    pub fn into_output_and_names(self) -> (ExactOpOutput, Vec<String>) {
        (self.output, self.names)
    }

    /// The ordinal of the face with this exact name.
    ///
    /// # Errors
    /// Returns [`NamingError::UnknownFace`] listing the existing names.
    pub fn face(&self, name: &str) -> Result<u32, NamingError> {
        self.names
            .iter()
            .position(|candidate| candidate == name)
            .map(ordinal)
            .ok_or_else(|| NamingError::UnknownFace {
                name: name.to_owned(),
                available: self.sorted_names(),
            })
    }

    /// The ordinal of the one edge between two named faces.
    ///
    /// # Errors
    /// Returns an error when a face is unknown, when the faces do not touch,
    /// or when they share more than one edge.
    pub fn edge(&self, first: &str, second: &str) -> Result<u32, NamingError> {
        edge_ordinal(&self.output, &self.names, first, second)
    }

    fn sorted_names(&self) -> Vec<String> {
        let mut names = self.names.clone();
        names.sort();
        names
    }
}

impl ExactBackend {
    /// Extrudes one closed profile at height `base_z_mm` along +Z.
    ///
    /// # Errors
    /// Returns an error for an invalid profile or name, or a failed extrusion.
    pub fn named_extrude(
        &self,
        profile: &[NamedSegment],
        base_z_mm: f64,
        height_mm: f64,
    ) -> Result<NamedBody, NamingError> {
        const OPERATION: &str = "named_extrude";
        let input = format!("{OPERATION}:{profile:?}:{base_z_mm}:{height_mm}");
        validate_length(height_mm, "height_mm", OPERATION, &input)?;
        let (segments, names) = counter_clockwise(profile, OPERATION, &input)?;
        let output = collect_output(
            ffi::named_prism_native(&flatten_planar_segments(&segments), base_z_mm, height_mm),
            OPERATION,
            &input,
            HistoryConfidence::Complete,
        )?;
        name_faces(output, OPERATION, |label| segment_name(label, &names))
    }

    /// Revolves one closed profile around an axis in its plane.
    ///
    /// # Errors
    /// Returns an error for an invalid profile or name, or a failed revolve.
    pub fn named_revolve(
        &self,
        profile: &[NamedSegment],
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    ) -> Result<NamedBody, NamingError> {
        const OPERATION: &str = "named_revolve";
        let input =
            format!("{OPERATION}:{profile:?}:{axis_start_mm:?}:{axis_end_mm:?}:{angle_degrees}");
        let (segments, names) = counter_clockwise(profile, OPERATION, &input)?;
        validate_general_revolve_profile(
            &segments,
            axis_start_mm,
            axis_end_mm,
            angle_degrees,
            &input,
        )?;
        let output = collect_output(
            ffi::named_revol_native(
                &flatten_planar_segments(&segments),
                axis_start_mm[0],
                axis_start_mm[1],
                axis_end_mm[0],
                axis_end_mm[1],
                angle_degrees,
            ),
            OPERATION,
            &input,
            HistoryConfidence::Complete,
        )?;
        name_faces(output, OPERATION, |label| segment_name(label, &names))
    }

    /// Fillets or chamfers the edges between the given pairs of named faces.
    ///
    /// # Errors
    /// Returns an error when an edge cannot be named or the finish fails.
    pub fn named_finish(
        &self,
        body: &NamedBody,
        edges: &[(&str, &str)],
        finish: EdgeFinish,
        amount_mm: f64,
    ) -> Result<NamedBody, NamingError> {
        self.named_finish_output(&body.output, &body.names, edges, finish, amount_mm)
    }

    /// Fillets or chamfers named edges on an output retained by an exact graph evaluator.
    ///
    /// # Errors
    /// Returns an error when the names do not cover the body or an edge cannot be resolved.
    pub fn named_finish_output(
        &self,
        output: &ExactOpOutput,
        names: &[String],
        edges: &[(&str, &str)],
        finish: EdgeFinish,
        amount_mm: f64,
    ) -> Result<NamedBody, NamingError> {
        const OPERATION: &str = "named_finish";
        let input = format!("{OPERATION}:{names:?}:{edges:?}:{finish:?}:{amount_mm}");
        validate_length(amount_mm, "amount_mm", OPERATION, &input)?;
        let ordinals = edges
            .iter()
            .map(|(first, second)| edge_ordinal(output, names, first, second))
            .collect::<Result<Vec<_>, _>>()?;
        let native = output.body.native.as_ref().ok_or_else(|| {
            NamingError::Geometry(GeometryError {
                code: GeometryErrorCode::NullResult,
                diagnostic: "Named body lost its owned native shape".to_owned(),
                operation: OPERATION,
                input_digest: String::new(),
                backend_fingerprint: crate::BACKEND_FINGERPRINT,
            })
        })?;
        let output = collect_output(
            ffi::named_finish_native(
                native,
                names,
                &ordinals,
                amount_mm,
                finish == EdgeFinish::Fillet,
            ),
            OPERATION,
            &input,
            HistoryConfidence::Complete,
        )?;
        name_faces(output, OPERATION, |label| Some(label.to_owned()))
    }

    /// Removes `tool` from `target`; faces the tool leaves are `tool_name.face`.
    ///
    /// # Errors
    /// Returns an error for an invalid tool name or a failed cut.
    pub fn named_cut(
        &self,
        target: &NamedBody,
        tool: &NamedBody,
        tool_name: &str,
    ) -> Result<NamedBody, NamingError> {
        self.named_cut_output(
            &target.output,
            &target.names,
            &tool.output,
            &tool.names,
            tool_name,
        )
    }

    /// Removes a named tool from a named target retained by an exact graph evaluator.
    ///
    /// # Errors
    /// Returns an error for incomplete names, an invalid tool name, or a failed cut.
    pub fn named_cut_output(
        &self,
        target: &ExactOpOutput,
        target_names: &[String],
        tool: &ExactOpOutput,
        tool_names: &[String],
        tool_name: &str,
    ) -> Result<NamedBody, NamingError> {
        const OPERATION: &str = "named_cut";
        if !valid_name(tool_name) {
            return Err(NamingError::InvalidName(tool_name.to_owned()));
        }
        let input = format!("{OPERATION}:{target_names:?}:{tool_names:?}:{tool_name}");
        let tool_labels = tool_names
            .iter()
            .map(|name| format!("{tool_name}.{name}"))
            .collect::<Vec<_>>();
        let target_native = output_native(target, OPERATION)?;
        let tool_native = output_native(tool, OPERATION)?;
        let output = collect_output(
            ffi::named_boolean_native(target_native, target_names, tool_native, &tool_labels, 0),
            OPERATION,
            &input,
            HistoryConfidence::Complete,
        )?;
        name_faces(output, OPERATION, |label| Some(label.to_owned()))
    }

    /// Push/Pull: moves one named planar face along its outward normal.
    ///
    /// # Errors
    /// Returns an error for an unknown or non-planar face or a failed offset.
    pub fn named_offset_face(
        &self,
        body: &NamedBody,
        face: &str,
        distance_mm: f64,
    ) -> Result<NamedBody, NamingError> {
        self.named_offset_face_output(&body.output, &body.names, face, distance_mm)
    }

    /// Moves one named face on an output retained by an exact graph evaluator.
    ///
    /// # Errors
    /// Returns an error for an unknown or non-planar face or a failed offset.
    pub fn named_offset_face_output(
        &self,
        output: &ExactOpOutput,
        names: &[String],
        face: &str,
        distance_mm: f64,
    ) -> Result<NamedBody, NamingError> {
        const OPERATION: &str = "named_offset_face";
        let input = format!("{OPERATION}:{names:?}:{face}:{distance_mm}");
        validate_length(distance_mm.abs(), "distance_mm", OPERATION, &input)?;
        let face_ordinal = names
            .iter()
            .position(|candidate| candidate == face)
            .map(ordinal)
            .ok_or_else(|| NamingError::UnknownFace {
                name: face.to_owned(),
                available: {
                    let mut available = names.to_vec();
                    available.sort();
                    available
                },
            })?;
        let output = collect_output(
            ffi::named_offset_face_native(
                output_native(output, OPERATION)?,
                names,
                face_ordinal,
                distance_mm,
            ),
            OPERATION,
            &input,
            HistoryConfidence::Complete,
        )?;
        name_faces(output, OPERATION, |label| Some(label.to_owned()))
    }
}

fn output_native<'a>(
    output: &'a ExactOpOutput,
    operation: &'static str,
) -> Result<&'a ffi::NativeOperationResult, NamingError> {
    output.body.native.as_ref().ok_or_else(|| {
        NamingError::Geometry(GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Named body lost its owned native shape".to_owned(),
            operation,
            input_digest: String::new(),
            backend_fingerprint: crate::BACKEND_FINGERPRINT,
        })
    })
}

fn edge_ordinal(
    output: &ExactOpOutput,
    names: &[String],
    first: &str,
    second: &str,
) -> Result<u32, NamingError> {
    let face = |name: &str| {
        names
            .iter()
            .position(|candidate| candidate == name)
            .map(ordinal)
            .ok_or_else(|| NamingError::UnknownFace {
                name: name.to_owned(),
                available: {
                    let mut available = names.to_vec();
                    available.sort();
                    available
                },
            })
    };
    let first_face = face(first)?;
    let second_face = face(second)?;
    let mut matches = Vec::new();
    let mut neighbours = BTreeSet::new();
    for edge in &output.body.topology.edges {
        let faces = edge
            .adjacent_face_ordinals
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if faces.contains(&first_face) {
            neighbours.extend(
                faces
                    .iter()
                    .filter(|face| **face != first_face)
                    .map(|face| names[*face as usize].clone()),
            );
        }
        if faces.len() == 2 && faces.contains(&first_face) && faces.contains(&second_face) {
            matches.push(edge.ordinal);
        }
    }
    match matches.as_slice() {
        [edge] => Ok(*edge),
        [] => Err(NamingError::UnknownEdge {
            first: first.to_owned(),
            second: second.to_owned(),
            neighbours_of_first: neighbours.into_iter().collect(),
        }),
        _ => Err(NamingError::AmbiguousEdge {
            first: first.to_owned(),
            second: second.to_owned(),
            count: matches.len(),
        }),
    }
}

fn ordinal(index: usize) -> u32 {
    u32::try_from(index).expect("face count fits in u32")
}

/// Whether a program may use `name` for a profile segment or a cutting tool.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name != "start"
        && name != "end"
        && !name.contains(['#', ',', '(', ')', '.', ':'])
}

/// The profile in counter-clockwise order with its names in the same order.
fn counter_clockwise(
    profile: &[NamedSegment],
    operation: &'static str,
    input: &str,
) -> Result<(Vec<PlanarProfileSegment>, Vec<String>), NamingError> {
    let mut seen = BTreeSet::new();
    for segment in profile {
        if !valid_name(&segment.name) || !seen.insert(segment.name.as_str()) {
            return Err(NamingError::InvalidName(segment.name.clone()));
        }
    }
    let segments = profile
        .iter()
        .map(|segment| segment.segment)
        .collect::<Vec<_>>();
    validate_mixed_profile(&segments, operation, input)?;
    let origin = planar_segment_endpoints(&segments[0]).0;
    let signed_area = segments
        .iter()
        .map(|segment| planar_segment_signed_area(segment, origin))
        .sum::<f64>();
    let names = profile
        .iter()
        .map(|segment| segment.name.clone())
        .collect::<Vec<_>>();
    if signed_area >= 0.0 {
        return Ok((segments, names));
    }
    Ok((
        reverse_planar_segments(&segments),
        names.into_iter().rev().collect(),
    ))
}

fn segment_name(label: &str, names: &[String]) -> Option<String> {
    match label {
        "start" | "end" => Some(label.to_owned()),
        _ => label
            .strip_prefix("seg:")?
            .parse::<usize>()
            .ok()
            .and_then(|index| names.get(index).cloned()),
    }
}

/// Turns the native per-face labels into final names: every face must have
/// one, faces merged from several sources join their names with `+`, and
/// faces sharing one name are split into `#1`, `#2`, ... in geometric order.
fn name_faces(
    output: ExactOpOutput,
    operation: &'static str,
    rename: impl Fn(&str) -> Option<String>,
) -> Result<NamedBody, NamingError> {
    let count = output.body.topology.face_count as usize;
    let mut sources = vec![BTreeSet::new(); count];
    for record in &output.topology_history {
        let (Some(face), Some(name)) = (
            record.output_face_ordinal,
            rename(&record.source_element_id),
        ) else {
            continue;
        };
        if let Some(set) = sources.get_mut(face as usize) {
            set.insert(name);
        }
    }
    let unnamed = sources
        .iter()
        .enumerate()
        .filter(|(_, names)| names.is_empty())
        .map(|(index, _)| ordinal(index))
        .collect::<Vec<_>>();
    if !unnamed.is_empty() {
        return Err(NamingError::UnnamedFaces {
            operation,
            ordinals: unnamed,
        });
    }
    let mut names = sources
        .into_iter()
        .map(|set| set.into_iter().collect::<Vec<_>>().join("+"))
        .collect::<Vec<_>>();
    let mut groups = BTreeMap::<String, Vec<usize>>::new();
    for (index, name) in names.iter().enumerate() {
        groups.entry(name.clone()).or_default().push(index);
    }
    let centroid = |index: usize| {
        let point = output
            .body
            .topology
            .faces
            .iter()
            .find(|face| face.ordinal as usize == index)
            .map(|face| face.centroid_mm)
            .expect("every face has evidence");
        // Micrometre grid so equal coordinates compare equal despite round-off.
        [point.x, point.y, point.z].map(|value| (value * 1_000.0).round() as i64)
    };
    for (name, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        members.sort_by_key(|index| centroid(*index));
        for (position, index) in members.into_iter().enumerate() {
            names[index] = format!("{name}#{}", position + 1);
        }
    }
    Ok(NamedBody { output, names })
}
