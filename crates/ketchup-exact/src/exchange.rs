use super::*;

pub const MAX_STEP_XDE_NODES: usize = 1_024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepXdePart {
    pub index: u32,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepXdeNode {
    pub id: u32,
    pub parent_id: Option<u32>,
    pub part_index: Option<u32>,
    pub name: String,
    pub name_from_source: bool,
    pub color: Option<[u8; 3]>,
    pub transform: [f64; 16],
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepXdeManifest {
    pub parts: Vec<StepXdePart>,
    pub nodes: Vec<StepXdeNode>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepXdeExportPart {
    pub path: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StepXdeExportNode {
    pub parent_id: Option<u32>,
    pub part_index: Option<u32>,
    pub name: String,
    pub color: Option<[u8; 3]>,
    pub transform: [f64; 16],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StepXdeManifestError {
    InvalidPath,
    Reader(String),
    Malformed,
    OutOfEnvelope,
}

impl std::fmt::Display for StepXdeManifestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath => formatter.write_str("STEP XDE path must not be empty"),
            Self::Reader(diagnostic) => formatter.write_str(diagnostic),
            Self::Malformed => formatter.write_str("STEP XDE manifest is malformed"),
            Self::OutOfEnvelope => {
                formatter.write_str("STEP XDE manifest exceeds the bounded envelope")
            }
        }
    }
}

impl std::error::Error for StepXdeManifestError {}

pub(super) fn decode_manifest_hex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

pub(super) fn parse_manifest_text(value: &str) -> Option<String> {
    let bytes = decode_manifest_hex(value)?;
    let text = String::from_utf8(bytes).ok()?;
    (!text.is_empty() && text.len() <= 1_024 && !text.chars().any(char::is_control)).then_some(text)
}

pub(super) fn parse_manifest_color(value: &str) -> Option<Option<[u8; 3]>> {
    if value == "-" {
        return Some(None);
    }
    let bytes = decode_manifest_hex(value)?;
    (bytes.len() == 3).then(|| Some([bytes[0], bytes[1], bytes[2]]))
}

pub(super) fn transform_is_rigid(matrix: &[f64; 16]) -> bool {
    if matrix.iter().any(|value| !value.is_finite())
        || matrix[12] != 0.0
        || matrix[13] != 0.0
        || matrix[14] != 0.0
        || matrix[15] != 1.0
    {
        return false;
    }
    let dot = |left: usize, right: usize| {
        matrix[left] * matrix[right]
            + matrix[4 + left] * matrix[4 + right]
            + matrix[8 + left] * matrix[8 + right]
    };
    (dot(0, 0) - 1.0).abs() <= ROUNDING
        && (dot(1, 1) - 1.0).abs() <= ROUNDING
        && (dot(2, 2) - 1.0).abs() <= ROUNDING
        && dot(0, 1).abs() <= ROUNDING
        && dot(0, 2).abs() <= ROUNDING
        && dot(1, 2).abs() <= ROUNDING
}

pub(super) fn parse_step_xde_manifest(raw: &str) -> Result<StepXdeManifest, StepXdeManifestError> {
    if let Some(diagnostic) = raw.strip_prefix("ERR\t") {
        let text = String::from_utf8(
            decode_manifest_hex(diagnostic).ok_or(StepXdeManifestError::Malformed)?,
        )
        .map_err(|_| StepXdeManifestError::Malformed)?;
        return Err(StepXdeManifestError::Reader(text));
    }
    let mut lines = raw.lines();
    if lines.next() != Some("KETCHUP_STEP_XDE_V1") {
        return Err(StepXdeManifestError::Malformed);
    }
    let mut parts = Vec::new();
    let mut nodes = Vec::new();
    for line in lines {
        let fields = line.split('\t').collect::<Vec<_>>();
        match fields.first().copied() {
            Some("P") if fields.len() == 5 => {
                if parts.len() >= MAX_STEP_XDE_NODES {
                    return Err(StepXdeManifestError::OutOfEnvelope);
                }
                let index = fields[1]
                    .parse::<u32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                if index as usize != parts.len() {
                    return Err(StepXdeManifestError::Malformed);
                }
                parts.push(StepXdePart {
                    index,
                    name: parse_manifest_text(fields[2]).ok_or(StepXdeManifestError::Malformed)?,
                    name_from_source: match fields[3] {
                        "0" => false,
                        "1" => true,
                        _ => return Err(StepXdeManifestError::Malformed),
                    },
                    color: parse_manifest_color(fields[4])
                        .ok_or(StepXdeManifestError::Malformed)?,
                });
            }
            Some("N") if fields.len() == 23 => {
                if nodes.len() >= MAX_STEP_XDE_NODES {
                    return Err(StepXdeManifestError::OutOfEnvelope);
                }
                let id = fields[1]
                    .parse::<u32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                if id as usize != nodes.len() {
                    return Err(StepXdeManifestError::Malformed);
                }
                let parent = fields[2]
                    .parse::<i32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                let parent_id = match parent {
                    -1 => None,
                    value if value >= 0 && (value as u32) < id => Some(value as u32),
                    _ => return Err(StepXdeManifestError::Malformed),
                };
                let part = fields[3]
                    .parse::<i32>()
                    .map_err(|_| StepXdeManifestError::Malformed)?;
                let part_index = match part {
                    -1 => None,
                    value if value >= 0 && (value as usize) < parts.len() => Some(value as u32),
                    _ => return Err(StepXdeManifestError::Malformed),
                };
                let mut transform = [0.0; 16];
                for (target, field) in transform.iter_mut().zip(&fields[7..]) {
                    *target = f64::from_bits(
                        u64::from_str_radix(field, 16)
                            .map_err(|_| StepXdeManifestError::Malformed)?,
                    );
                }
                if !transform_is_rigid(&transform) {
                    return Err(StepXdeManifestError::Malformed);
                }
                nodes.push(StepXdeNode {
                    id,
                    parent_id,
                    part_index,
                    name: parse_manifest_text(fields[4]).ok_or(StepXdeManifestError::Malformed)?,
                    name_from_source: match fields[5] {
                        "0" => false,
                        "1" => true,
                        _ => return Err(StepXdeManifestError::Malformed),
                    },
                    color: parse_manifest_color(fields[6])
                        .ok_or(StepXdeManifestError::Malformed)?,
                    transform,
                });
            }
            _ => return Err(StepXdeManifestError::Malformed),
        }
    }
    if parts.is_empty()
        || nodes.is_empty()
        || !nodes.iter().any(|node| node.parent_id.is_none())
        || parts
            .iter()
            .any(|part| !nodes.iter().any(|node| node.part_index == Some(part.index)))
    {
        return Err(StepXdeManifestError::Malformed);
    }
    Ok(StepXdeManifest { parts, nodes })
}

impl ExactBackend {
    pub fn exception_probe(&self) -> Result<ExactOpOutput, GeometryError> {
        collect_output(
            ffi::exception_probe_native(),
            "exception_probe",
            "intentional",
            HistoryConfidence::None,
        )
    }

    pub fn import_step(&self, path: &str) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_step:{path}");
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_step",
                &input,
                "STEP path must not be empty".to_owned(),
            ));
        }
        collect_output(
            ffi::import_step_native(path),
            "import_step",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn import_step_solid(
        &self,
        path: &str,
        solid_ordinal: u32,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_step_solid:{path}:{solid_ordinal}");
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_step_solid",
                &input,
                "STEP path must not be empty".to_owned(),
            ));
        }
        collect_output(
            ffi::import_step_solid_native(path, solid_ordinal),
            "import_step_solid",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn import_step_xde_part(
        &self,
        path: &str,
        part_index: u32,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_step_xde_part:{path}:{part_index}");
        if path.trim().is_empty() || part_index as usize >= MAX_STEP_XDE_NODES {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_step_xde_part",
                &input,
                "STEP XDE path or part index is invalid".to_owned(),
            ));
        }
        collect_output(
            ffi::import_step_xde_part_native(path, part_index),
            "import_step_xde_part",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn step_xde_manifest(&self, path: &str) -> Result<StepXdeManifest, StepXdeManifestError> {
        if path.trim().is_empty() {
            return Err(StepXdeManifestError::InvalidPath);
        }
        parse_step_xde_manifest(&ffi::step_xde_manifest_native(path))
    }

    pub fn export_step_xde_assembly(
        &self,
        parts: &[StepXdeExportPart],
        nodes: &[StepXdeExportNode],
        path: &str,
    ) -> Result<(), GeometryError> {
        self.export_xde_assembly(parts, nodes, path, false)
    }

    pub fn export_iges_xde_assembly(
        &self,
        parts: &[StepXdeExportPart],
        nodes: &[StepXdeExportNode],
        path: &str,
    ) -> Result<(), GeometryError> {
        self.export_xde_assembly(parts, nodes, path, true)
    }

    pub(super) fn export_xde_assembly(
        &self,
        parts: &[StepXdeExportPart],
        nodes: &[StepXdeExportNode],
        path: &str,
        iges: bool,
    ) -> Result<(), GeometryError> {
        let operation = if iges {
            "export_iges_xde_assembly"
        } else {
            "export_step_xde_assembly"
        };
        let input = format!("{operation}:{}:{}:{path}", parts.len(), nodes.len());
        let invalid = || {
            parameter_error(
                GeometryErrorCode::InvalidParameter,
                operation,
                &input,
                "XDE assembly manifest is malformed or outside the bounded envelope".to_owned(),
            )
        };
        if path.trim().is_empty()
            || parts.is_empty()
            || parts.len() > MAX_STEP_XDE_NODES
            || nodes.is_empty()
            || nodes.len() > MAX_STEP_XDE_NODES
            || parts.iter().any(|part| {
                part.path.trim().is_empty()
                    || part.name.is_empty()
                    || part.name.len() > 4_096
                    || part.name.chars().any(char::is_control)
            })
        {
            return Err(invalid());
        }
        let mut used_parts = vec![false; parts.len()];
        for (index, node) in nodes.iter().enumerate() {
            if node.name.is_empty()
                || node.name.len() > 4_096
                || node.name.chars().any(char::is_control)
                || node
                    .parent_id
                    .is_some_and(|parent| parent as usize >= index)
                || node
                    .part_index
                    .is_some_and(|part| part as usize >= parts.len())
                || node
                    .parent_id
                    .is_some_and(|parent| nodes[parent as usize].part_index.is_some())
                || !transform_is_rigid(&node.transform)
            {
                return Err(invalid());
            }
            if let Some(part) = node.part_index {
                used_parts[part as usize] = true;
            }
            let mut depth = 0usize;
            let mut parent = node.parent_id;
            while let Some(parent_id) = parent {
                depth += 1;
                if depth > 64 {
                    return Err(invalid());
                }
                parent = nodes[parent_id as usize].parent_id;
            }
        }
        if used_parts.iter().any(|used| !used) {
            return Err(invalid());
        }
        let encode = |value: &str| {
            const DIGITS: &[u8; 16] = b"0123456789abcdef";
            let mut output = String::with_capacity(value.len() * 2);
            for byte in value.as_bytes() {
                output.push(DIGITS[(byte >> 4) as usize] as char);
                output.push(DIGITS[(byte & 0x0f) as usize] as char);
            }
            output
        };
        let mut manifest = String::from("KETCHUP_STEP_XDE_EXPORT_V1\n");
        for part in parts {
            use std::fmt::Write as _;
            writeln!(
                manifest,
                "P\t{}\t{}",
                encode(&part.path),
                encode(&part.name)
            )
            .expect("writing to String cannot fail");
        }
        for node in nodes {
            use std::fmt::Write as _;
            let parent = node.parent_id.map_or(-1_i64, i64::from);
            let part = node.part_index.map_or(-1_i64, i64::from);
            let color = node.color.map_or_else(
                || "-".to_owned(),
                |[red, green, blue]| format!("{red:02x}{green:02x}{blue:02x}"),
            );
            write!(
                manifest,
                "N\t{parent}\t{part}\t{}\t{color}",
                encode(&node.name)
            )
            .expect("writing to String cannot fail");
            for value in node.transform {
                write!(manifest, "\t{:016x}", value.to_bits())
                    .expect("writing to String cannot fail");
            }
            manifest.push('\n');
        }
        let diagnostic = if iges {
            ffi::export_iges_xde_assembly_native(&manifest, path)
        } else {
            ffi::export_step_xde_assembly_native(&manifest, path)
        };
        if diagnostic.is_empty() {
            Ok(())
        } else {
            Err(GeometryError {
                code: GeometryErrorCode::BackendException,
                diagnostic,
                operation,
                input_digest: stable_digest(&input),
                backend_fingerprint: BACKEND_FINGERPRINT,
            })
        }
    }

    #[must_use]
    pub fn step_length_unit_name(&self, path: &str) -> Option<String> {
        let unit = ffi::step_length_unit_native(path);
        (!unit.is_empty()).then_some(unit)
    }

    pub fn import_iges(&self, path: &str) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_iges:{path}");
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_iges",
                &input,
                "IGES path must not be empty".to_owned(),
            ));
        }
        collect_output(
            ffi::import_iges_native(path),
            "import_iges",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn import_iges_xde_part(
        &self,
        path: &str,
        part_index: u32,
    ) -> Result<ExactOpOutput, GeometryError> {
        let input = format!("import_iges_xde_part:{path}:{part_index}");
        if path.trim().is_empty() || part_index as usize >= MAX_STEP_XDE_NODES {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "import_iges_xde_part",
                &input,
                "IGES XDE path or part index is invalid".to_owned(),
            ));
        }
        collect_output(
            ffi::import_iges_xde_part_native(path, part_index),
            "import_iges_xde_part",
            &input,
            HistoryConfidence::None,
        )
    }

    pub fn iges_xde_manifest(&self, path: &str) -> Result<StepXdeManifest, StepXdeManifestError> {
        if path.trim().is_empty() {
            return Err(StepXdeManifestError::InvalidPath);
        }
        parse_step_xde_manifest(&ffi::iges_xde_manifest_native(path))
    }

    #[must_use]
    pub fn iges_length_unit_name(&self, path: &str) -> Option<String> {
        let unit = ffi::iges_length_unit_native(path);
        (!unit.is_empty()).then_some(unit)
    }
}

impl ExactBackend {
    pub fn export_step(&self, body: &ExactBody, path: &str) -> Result<(), GeometryError> {
        let input = format!("export_step:{}:{path}", body.result_fingerprint);
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "export_step",
                &input,
                "STEP path must not be empty".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "export_step",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let diagnostic = ffi::export_step_native(native, path);
        if diagnostic.is_empty() {
            Ok(())
        } else {
            Err(GeometryError {
                code: GeometryErrorCode::BackendException,
                diagnostic,
                operation: "export_step",
                input_digest: stable_digest(&input),
                backend_fingerprint: BACKEND_FINGERPRINT,
            })
        }
    }

    pub fn export_iges(&self, body: &ExactBody, path: &str) -> Result<(), GeometryError> {
        let input = format!("export_iges:{}:{path}", body.result_fingerprint);
        if path.trim().is_empty() {
            return Err(parameter_error(
                GeometryErrorCode::InvalidParameter,
                "export_iges",
                &input,
                "IGES path must not be empty".to_owned(),
            ));
        }
        let native = body.native.as_ref().ok_or_else(|| GeometryError {
            code: GeometryErrorCode::NullResult,
            diagnostic: "Exact body lost its owned native shape".to_owned(),
            operation: "export_iges",
            input_digest: stable_digest(&input),
            backend_fingerprint: BACKEND_FINGERPRINT,
        })?;
        let diagnostic = ffi::export_iges_native(native, path);
        if diagnostic.is_empty() {
            Ok(())
        } else {
            Err(GeometryError {
                code: GeometryErrorCode::BackendException,
                diagnostic,
                operation: "export_iges",
                input_digest: stable_digest(&input),
                backend_fingerprint: BACKEND_FINGERPRINT,
            })
        }
    }
}
