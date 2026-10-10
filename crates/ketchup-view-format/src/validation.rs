//! Manifest checks against the embedded GLB, run on every read and write, so a
//! package that passes can be shown without further trust in its author.

use crate::{
    Camera, InstancePath, LocalDimensions, Manifest, PathStep, Projection, glb::Info, reject,
};
use ketchup_rejection::Rejection;
use std::collections::BTreeSet;

impl Manifest {
    /// Check identity, references, limits and text against the GLB nodes in `info`.
    pub(crate) fn validate(&self, info: &Info) -> Result<(), Rejection> {
        if self.source.document_id == 0
            || self.source.canonical_digest.len() != 16
            || !self
                .source
                .canonical_digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            || self.source.canonical_digest != info.source_digest
        {
            return Err(reject(
                "source",
                "Manifest source identity does not match the geometry.",
            ));
        }
        if self.definitions.len() > 100_000
            || self.occurrences.len() > 100_000
            || self.scenes.len() > 1_000
            || self.dimensions.len() > 100_000
        {
            return Err(reject("manifest", "The manifest exceeds its item limits."));
        }
        let mut definitions = BTreeSet::new();
        for definition in &self.definitions {
            if definition.id == 0 || !definitions.insert(definition.id) {
                return Err(reject(
                    "definitions",
                    "Definition IDs must be unique and nonzero.",
                ));
            }
            text(&definition.name, 4_096)?;
            optional_text(&definition.note, 65_536)?;
            optional_text(&definition.material, 4_096)?;
            if let Some(dimensions) = &definition.local_dimensions {
                match dimensions {
                    LocalDimensions::LocalBounds { min_mm, max_mm } => {
                        point(min_mm)?;
                        point(max_mm)?;
                        if (0..3).any(|i| max_mm[i] < min_mm[i]) {
                            return Err(reject("local_dimensions", "Local bounds are reversed."));
                        }
                    }
                    LocalDimensions::AuthoredAxes { size_mm } => {
                        point(size_mm)?;
                        if size_mm.iter().any(|&v| v < 0.0) {
                            return Err(reject(
                                "local_dimensions",
                                "Local dimensions cannot be negative.",
                            ));
                        }
                    }
                }
            }
        }
        let mut parents = vec![None; info.nodes.len()];
        for (parent, node) in info.nodes.iter().enumerate() {
            for &child in &node.children {
                parents[child] = Some(parent);
            }
        }
        let mut paths = BTreeSet::new();
        let mut nodes = BTreeSet::new();
        for occurrence in &self.occurrences {
            text(&occurrence.name, 4_096)?;
            validate_path(&occurrence.path)?;
            if !paths.insert(&occurrence.path)
                || !nodes.insert(occurrence.glb_node)
                || !definitions.contains(&occurrence.definition_id)
            {
                return Err(reject(
                    "occurrences",
                    "Duplicate occurrence or missing definition.",
                ));
            }
            validate_occurrence(occurrence, info, &parents, &definitions)?;
            optional_text(&occurrence.note, 65_536)?;
            optional_text(&occurrence.material, 4_096)?;
            if occurrence.attributes.len() > 1_024 {
                return Err(reject("attributes", "A component has too many attributes."));
            }
            for (key, value) in &occurrence.attributes {
                text(key, 4_096)?;
                text(value, 65_536)?;
            }
        }
        let mut dimensions = BTreeSet::new();
        for dimension in &self.dimensions {
            if dimension.id == 0 || !dimensions.insert(dimension.id) {
                return Err(reject(
                    "dimensions",
                    "Dimension IDs must be unique and nonzero.",
                ));
            }
            for anchor in [&dimension.from, &dimension.to] {
                point(&anchor.point_mm)?;
                if anchor
                    .occurrence
                    .as_ref()
                    .is_some_and(|path| !paths.contains(path))
                {
                    return Err(reject(
                        "dimensions.anchor",
                        "A dimension refers to a missing occurrence.",
                    ));
                }
            }
            point(&dimension.label_offset_mm)?;
            finite(dimension.value_mm)?;
            if dimension.value_mm < 0.0 {
                return Err(reject(
                    "dimensions.value",
                    "Dimension values cannot be negative.",
                ));
            }
        }
        let mut scenes = BTreeSet::new();
        for scene in &self.scenes {
            if scene.id == 0 || !scenes.insert(scene.id) {
                return Err(reject("scenes", "Scene IDs must be unique and nonzero."));
            }
            text(&scene.name, 4_096)?;
            scene.camera.validate()?;
            if scene.hidden.iter().any(|path| !paths.contains(path))
                || scene
                    .visible_dimensions
                    .iter()
                    .any(|id| !dimensions.contains(id))
            {
                return Err(reject(
                    "scenes.references",
                    "Scene visibility refers to a missing occurrence or dimension.",
                ));
            }
            if let Some(section) = &scene.section {
                unit_direction(&section.normal)?;
                finite(section.offset_mm)?;
            }
        }
        if self.start_scene.is_some_and(|id| !scenes.contains(&id)) {
            return Err(reject("start_scene", "The starting scene is missing."));
        }
        Ok(())
    }
}

/// One occurrence: its GLB node, definition, path and text fields.
fn validate_occurrence(
    occurrence: &crate::Occurrence,
    info: &Info,
    parents: &[Option<usize>],
    definitions: &BTreeSet<u64>,
) -> Result<(), Rejection> {
    let mut index = occurrence.glb_node;
    let mut path = occurrence.path.clone();
    let mut definition_id = occurrence.definition_id;
    loop {
        let node = info
            .nodes
            .get(index)
            .ok_or_else(|| reject("occurrences.glb_node", "Missing geometry node."))?;
        let metadata = &node.extras;
        if metadata
            .get("ketchupInstancePath")
            .and_then(serde_json::Value::as_str)
            != Some(path.glb_key().as_str())
        {
            return Err(reject(
                "occurrences.path",
                "The source path does not match its GLB ancestry.",
            ));
        }
        let step = path.steps.pop();
        let is_group = matches!(step, Some(PathStep::Group { .. }));
        if metadata
            .get("ketchupEntity")
            .and_then(serde_json::Value::as_str)
            != Some(if is_group { "group" } else { "occurrence" })
            || (!is_group
                && metadata
                    .get("ketchupDefinitionId")
                    .and_then(serde_json::Value::as_u64)
                    != Some(definition_id))
        {
            return Err(reject(
                "occurrences.definition_id",
                "The source definition does not match its GLB occurrence.",
            ));
        }
        let Some(step) = step else {
            break;
        };
        let owner = match step {
            PathStep::Group {
                owner_definition_id,
                ..
            } => {
                if owner_definition_id != definition_id {
                    return Err(reject(
                        "occurrences.path",
                        "A local group belongs to a different definition.",
                    ));
                }
                owner_definition_id
            }
            PathStep::Occurrence {
                owner_definition_id,
                ..
            } => owner_definition_id,
        };
        if !definitions.contains(&owner)
            || metadata
                .get("ketchupOwnerDefinitionId")
                .and_then(serde_json::Value::as_u64)
                != Some(owner)
        {
            return Err(reject(
                "occurrences.path",
                "A path step has a missing or incorrect owning definition.",
            ));
        }
        definition_id = owner;
        index = parents[index].ok_or_else(|| {
            reject(
                "occurrences.path",
                "A nested occurrence is missing its parent.",
            )
        })?;
    }
    Ok(())
}

impl InstancePath {
    /// The node name the GLB exporter gives this path, e.g. `occurrence:10/group:2`.
    pub fn glb_key(&self) -> String {
        let mut key = format!("occurrence:{}", self.root_occurrence_id);
        for step in &self.steps {
            match step {
                PathStep::Group { local_id, .. } => key.push_str(&format!("/group:{local_id}")),
                PathStep::Occurrence { local_id, .. } => {
                    key.push_str(&format!("/occurrence:{local_id}"))
                }
            }
        }
        key
    }
}

/// Nonzero ids and a bounded depth.
fn validate_path(path: &InstancePath) -> Result<(), Rejection> {
    if path.root_occurrence_id == 0
        || path.steps.len() > 128
        || path.steps.iter().any(|step| match step {
            PathStep::Group {
                owner_definition_id,
                local_id,
            }
            | PathStep::Occurrence {
                owner_definition_id,
                local_id,
            } => *owner_definition_id == 0 || *local_id == 0,
        })
    {
        return Err(reject(
            "path",
            "Occurrence path has zero IDs or exceeds 128 steps.",
        ));
    }
    Ok(())
}

impl Camera {
    /// Finite points, a usable up direction and a positive projection.
    pub fn validate(&self) -> Result<(), Rejection> {
        point(&self.eye_mm)?;
        point(&self.target_mm)?;
        unit_direction(&self.up)?;
        let forward = std::array::from_fn::<_, 3, _>(|i| self.target_mm[i] - self.eye_mm[i]);
        let length2: f64 = forward.iter().map(|v| v * v).sum();
        let dot: f64 = forward.iter().zip(self.up).map(|(a, b)| a * b).sum();
        if length2 < 1e-12 || dot * dot / length2 > 1.0 - 1e-8 {
            return Err(reject(
                "camera",
                "Camera needs distinct eye/target and a nonparallel up vector.",
            ));
        }
        match self.projection {
            Projection::Orthographic { short_span_mm } => {
                finite(short_span_mm)?;
                if short_span_mm <= 0.0 {
                    return Err(reject("camera.span", "Camera span must be positive."));
                }
            }
            Projection::Perspective {
                short_fov_radians,
                lens_shift_short,
            } => {
                for shift in lens_shift_short {
                    finite(shift)?;
                }
                if !short_fov_radians.is_finite() || !(0.001..3.0).contains(&short_fov_radians) {
                    return Err(reject(
                        "camera.fov",
                        "Perspective FOV must be between 0.001 and 3 radians.",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Returns orthographic width/height in mm, or perspective width/height
    /// tangents of half-FOV. These are viewport independent, not pixel scales.
    pub fn framing(&self, aspect: f64) -> Result<[f64; 2], Rejection> {
        self.validate()?;
        if !aspect.is_finite() || !(0.01..=100.0).contains(&aspect) {
            return Err(reject(
                "camera.aspect",
                "Aspect ratio must be finite between 0.01 and 100.",
            ));
        }
        let short = match self.projection {
            Projection::Orthographic { short_span_mm } => short_span_mm,
            Projection::Perspective {
                short_fov_radians, ..
            } => (short_fov_radians / 2.0).tan(),
        };
        Ok(if aspect >= 1.0 {
            [short * aspect, short]
        } else {
            [short, short / aspect]
        })
    }
}

fn finite(v: f64) -> Result<(), Rejection> {
    if !v.is_finite() || v.abs() > 1e9 {
        return Err(reject(
            "number",
            "Source lengths must be finite within ±1e9 mm.",
        ));
    }
    Ok(())
}
fn point(v: &[f64; 3]) -> Result<(), Rejection> {
    for &n in v {
        finite(n)?;
    }
    Ok(())
}
fn unit_direction(v: &[f64; 3]) -> Result<(), Rejection> {
    if v.iter().any(|n| !n.is_finite()) || (v.iter().map(|n| n * n).sum::<f64>() - 1.0).abs() > 1e-6
    {
        return Err(reject(
            "direction",
            "Camera/section directions must be finite unit vectors.",
        ));
    }
    Ok(())
}
/// Bounded text without control characters.
fn text(value: &str, max: usize) -> Result<(), Rejection> {
    if value.len() > max || value.contains('\0') {
        return Err(reject(
            "text",
            "Text exceeds its length limit or contains NUL.",
        ));
    }
    Ok(())
}
fn optional_text(value: &Option<String>, max: usize) -> Result<(), Rejection> {
    if let Some(value) = value {
        text(value, max)?;
    }
    Ok(())
}
