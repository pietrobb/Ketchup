use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use super::{
    ImportDiagnostic, ImportDiagnosticSeverity, ImportFormat, ImportLengthUnit, ImportOutputRef,
    ImportReceipt, ImportUnitAuthority, ImportUnitDecision,
};
use crate::document::{
    CanonicalCommand, CommandBatch, DefinitionId, FeatureId, FeatureKind, MESH_BODY_SCHEMA_V1,
    MeshAuthority, MeshBodySpec, OccurrenceId, Snapshot, Transform,
};

pub const SKETCHUP_SCENE_SCHEMA_V1: &str = "ketchup.sketchup-scene.v1";
pub const SKETCHUP_SCENE_PARSER_ID: &str = "ketchup-sketchup-scene";
pub const SKETCHUP_SCENE_PARSER_VERSION: &str = "1";
pub const MAX_SKETCHUP_SCENE_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DEFINITIONS: usize = 128;
const MAX_INSTANCES: usize = 512;
const MAX_TOTAL_VERTICES: usize = 200_000;
const MAX_TOTAL_TRIANGLES: usize = 400_000;
const MAX_VERTICES_PER_DEFINITION: usize = 100_000;
const MAX_TRIANGLES_PER_DEFINITION: usize = 200_000;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_ABS_MM: f64 = 1_000_000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedSketchupScene {
    definitions: Vec<ParsedDefinition>,
    instances: Vec<ParsedInstance>,
    diagnostics: Vec<ImportDiagnostic>,
    triangle_count: usize,
}

impl ParsedSketchupScene {
    #[must_use]
    pub fn definition_count(&self) -> usize {
        self.definitions.len()
    }

    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    #[must_use]
    pub const fn triangle_count(&self) -> usize {
        self.triangle_count
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ImportDiagnostic] {
        &self.diagnostics
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedDefinition {
    source_id: String,
    name: String,
    vertices_mm: Vec<[f64; 3]>,
    triangles: Vec<[u32; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
struct ParsedInstance {
    definition: String,
    name: String,
    transform: Transform,
    visible: bool,
}

struct BoundedVec<T, const LIMIT: usize>(Vec<T>);

impl<T, const LIMIT: usize> std::ops::Deref for BoundedVec<T, LIMIT> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T, const LIMIT: usize> IntoIterator for BoundedVec<T, LIMIT> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T, const LIMIT: usize> IntoIterator for &'a BoundedVec<T, LIMIT> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'de, T: Deserialize<'de>, const LIMIT: usize> Deserialize<'de> for BoundedVec<T, LIMIT> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BoundedVisitor<T, const LIMIT: usize>(std::marker::PhantomData<T>);

        impl<'de, T: Deserialize<'de>, const LIMIT: usize> Visitor<'de> for BoundedVisitor<T, LIMIT> {
            type Value = BoundedVec<T, LIMIT>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "an array with at most {LIMIT} items")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                if sequence.size_hint().is_some_and(|size| size > LIMIT) {
                    return Err(A::Error::custom(format!(
                        "bounded array exceeds limit {LIMIT}"
                    )));
                }
                let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(LIMIT));
                while let Some(value) = sequence.next_element()? {
                    if values.len() == LIMIT {
                        return Err(A::Error::custom(format!(
                            "bounded array exceeds limit {LIMIT}"
                        )));
                    }
                    values.push(value);
                }
                Ok(BoundedVec(values))
            }
        }

        deserializer.deserialize_seq(BoundedVisitor(std::marker::PhantomData))
    }
}

struct DefinitionFiles(Vec<DefinitionFile>);

impl std::ops::Deref for DefinitionFiles {
    type Target = [DefinitionFile];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IntoIterator for DefinitionFiles {
    type Item = DefinitionFile;
    type IntoIter = std::vec::IntoIter<DefinitionFile>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'de> Deserialize<'de> for DefinitionFiles {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct DefinitionsVisitor;

        impl<'de> Visitor<'de> for DefinitionsVisitor {
            type Value = DefinitionFiles;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a bounded array of SketchUp definitions")
            }

            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                let mut vertices = 0_usize;
                let mut triangles = 0_usize;
                while let Some(value) = sequence.next_element::<DefinitionFile>()? {
                    if values.len() == MAX_DEFINITIONS {
                        return Err(A::Error::custom("too many definitions"));
                    }
                    vertices = vertices
                        .checked_add(value.vertices.0.len())
                        .ok_or_else(|| A::Error::custom("too many vertices"))?;
                    triangles = triangles
                        .checked_add(value.triangles.0.len())
                        .ok_or_else(|| A::Error::custom("too many triangles"))?;
                    if vertices > MAX_TOTAL_VERTICES {
                        return Err(A::Error::custom("too many total vertices"));
                    }
                    if triangles > MAX_TOTAL_TRIANGLES {
                        return Err(A::Error::custom("too many total triangles"));
                    }
                    values.push(value);
                }
                Ok(DefinitionFiles(values))
            }
        }

        deserializer.deserialize_seq(DefinitionsVisitor)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SceneFile {
    schema: String,
    units: String,
    definitions: DefinitionFiles,
    instances: BoundedVec<InstanceFile, MAX_INSTANCES>,
    metadata: MetadataFile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DefinitionFile {
    id: String,
    name: String,
    vertices: BoundedVec<[f64; 3], MAX_VERTICES_PER_DEFINITION>,
    triangles: BoundedVec<[u32; 3], MAX_TRIANGLES_PER_DEFINITION>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceFile {
    definition: String,
    name: String,
    transform: [f64; 16],
    visible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataFile {
    material_assignments: u32,
    textures: u32,
    tags: u32,
    scenes: u32,
    unsupported_entities: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SketchupSceneImportError {
    Empty,
    SourceTooLarge,
    InvalidUtf8,
    InvalidJson,
    UnsupportedSchema,
    UnsupportedUnits,
    EmptyScene,
    TooManyDefinitions,
    TooManyInstances,
    TooManyVertices,
    TooManyTriangles,
    InvalidText,
    DuplicateDefinition,
    MissingDefinition,
    InvalidGeometry,
    InvalidTransform,
    InvalidSourceIdentity,
    IdSpaceExhausted,
}

impl fmt::Display for SketchupSceneImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "SketchUp scene package is empty",
            Self::SourceTooLarge => "SketchUp scene package exceeds the bounded 32 MiB envelope",
            Self::InvalidUtf8 => "SketchUp scene package is not valid UTF-8",
            Self::InvalidJson => {
                "SketchUp scene package JSON is malformed or contains unknown fields"
            }
            Self::UnsupportedSchema => "SketchUp scene package schema is unsupported",
            Self::UnsupportedUnits => {
                "SketchUp scene package must declare SketchUp internal inches"
            }
            Self::EmptyScene => "SketchUp scene package contains no importable solid instances",
            Self::TooManyDefinitions => "SketchUp scene package exceeds the 128-definition limit",
            Self::TooManyInstances => "SketchUp scene package exceeds the 512-instance limit",
            Self::TooManyVertices => "SketchUp scene package exceeds the bounded vertex envelope",
            Self::TooManyTriangles => {
                "SketchUp scene package exceeds the bounded triangle envelope"
            }
            Self::InvalidText => "SketchUp scene package contains invalid identity text",
            Self::DuplicateDefinition => "SketchUp scene package contains duplicate definition IDs",
            Self::MissingDefinition => {
                "SketchUp scene package instance references a missing definition"
            }
            Self::InvalidGeometry => "SketchUp scene package contains invalid mesh geometry",
            Self::InvalidTransform => {
                "SketchUp scene package contains an invalid instance transform"
            }
            Self::InvalidSourceIdentity => {
                "SketchUp scene package source name or provenance is invalid"
            }
            Self::IdSpaceExhausted => "canonical import ID space is exhausted",
        })
    }
}

impl std::error::Error for SketchupSceneImportError {}

fn classify_json_error(error: &serde_json::Error) -> SketchupSceneImportError {
    let message = error.to_string();
    if message.contains("too many definitions") {
        SketchupSceneImportError::TooManyDefinitions
    } else if message.contains("bounded array exceeds limit 512") {
        SketchupSceneImportError::TooManyInstances
    } else if message.contains("bounded array exceeds limit 100000")
        || message.contains("too many vertices")
        || message.contains("too many total vertices")
    {
        SketchupSceneImportError::TooManyVertices
    } else if message.contains("bounded array exceeds limit 200000")
        || message.contains("too many triangles")
        || message.contains("too many total triangles")
    {
        SketchupSceneImportError::TooManyTriangles
    } else {
        SketchupSceneImportError::InvalidJson
    }
}

pub fn inspect_sketchup_scene(
    source: &[u8],
) -> Result<ParsedSketchupScene, SketchupSceneImportError> {
    if source.is_empty() {
        return Err(SketchupSceneImportError::Empty);
    }
    if source.len() as u64 > MAX_SKETCHUP_SCENE_SOURCE_BYTES {
        return Err(SketchupSceneImportError::SourceTooLarge);
    }
    let text = std::str::from_utf8(source).map_err(|_| SketchupSceneImportError::InvalidUtf8)?;
    let file: SceneFile =
        serde_json::from_str(text).map_err(|error| classify_json_error(&error))?;
    if file.schema != SKETCHUP_SCENE_SCHEMA_V1 {
        return Err(SketchupSceneImportError::UnsupportedSchema);
    }
    if file.units != "inch" {
        return Err(SketchupSceneImportError::UnsupportedUnits);
    }
    if file.definitions.is_empty() || file.instances.is_empty() {
        return Err(SketchupSceneImportError::EmptyScene);
    }
    if file.definitions.len() > MAX_DEFINITIONS {
        return Err(SketchupSceneImportError::TooManyDefinitions);
    }
    if file.instances.len() > MAX_INSTANCES {
        return Err(SketchupSceneImportError::TooManyInstances);
    }

    let mut ids = BTreeSet::new();
    let mut definitions = Vec::with_capacity(file.definitions.len());
    let mut total_vertices = 0_usize;
    let mut total_triangles = 0_usize;
    for definition in file.definitions {
        validate_text(&definition.id)?;
        validate_text(&definition.name)?;
        if !ids.insert(definition.id.clone()) {
            return Err(SketchupSceneImportError::DuplicateDefinition);
        }
        if !(4..=MAX_VERTICES_PER_DEFINITION).contains(&definition.vertices.len()) {
            return Err(SketchupSceneImportError::TooManyVertices);
        }
        if !(4..=MAX_TRIANGLES_PER_DEFINITION).contains(&definition.triangles.len()) {
            return Err(SketchupSceneImportError::TooManyTriangles);
        }
        total_vertices = total_vertices
            .checked_add(definition.vertices.len())
            .ok_or(SketchupSceneImportError::TooManyVertices)?;
        total_triangles = total_triangles
            .checked_add(definition.triangles.len())
            .ok_or(SketchupSceneImportError::TooManyTriangles)?;
        if total_vertices > MAX_TOTAL_VERTICES {
            return Err(SketchupSceneImportError::TooManyVertices);
        }
        if total_triangles > MAX_TOTAL_TRIANGLES {
            return Err(SketchupSceneImportError::TooManyTriangles);
        }
        let vertices_mm = definition
            .vertices
            .into_iter()
            .map(|vertex| {
                let scaled = vertex.map(|coordinate| coordinate * 25.4);
                if scaled
                    .iter()
                    .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > MAX_ABS_MM)
                {
                    Err(SketchupSceneImportError::InvalidGeometry)
                } else {
                    Ok(scaled)
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut triangles_seen = BTreeSet::new();
        for triangle in &definition.triangles {
            if triangle[0] == triangle[1]
                || triangle[1] == triangle[2]
                || triangle[0] == triangle[2]
                || triangle
                    .iter()
                    .any(|index| *index as usize >= vertices_mm.len())
            {
                return Err(SketchupSceneImportError::InvalidGeometry);
            }
            let mut canonical = *triangle;
            canonical.sort_unstable();
            if !triangles_seen.insert(canonical) {
                return Err(SketchupSceneImportError::InvalidGeometry);
            }
        }
        definitions.push(ParsedDefinition {
            source_id: definition.id,
            name: definition.name,
            vertices_mm,
            triangles: definition.triangles.0,
        });
    }

    let instance_count = file.instances.len();
    let mut instances = Vec::with_capacity(instance_count);
    for instance in file.instances {
        validate_text(&instance.definition)?;
        validate_text(&instance.name)?;
        if !ids.contains(&instance.definition) {
            return Err(SketchupSceneImportError::MissingDefinition);
        }
        let mut matrix = instance.transform;
        for index in [3, 7, 11] {
            matrix[index] *= 25.4;
        }
        let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
            - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
            + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
        if matrix.iter().any(|value| !value.is_finite())
            || [0, 1, 2, 4, 5, 6, 8, 9, 10]
                .into_iter()
                .any(|index| matrix[index].abs() > MAX_ABS_MM)
            || !determinant.is_finite()
            || determinant.abs() <= 1.0e-12
        {
            return Err(SketchupSceneImportError::InvalidTransform);
        }
        let definition = definitions
            .iter()
            .find(|definition| definition.source_id == instance.definition)
            .ok_or(SketchupSceneImportError::MissingDefinition)?;
        if definition.vertices_mm.iter().any(|vertex| {
            (0..3).any(|row| {
                let coordinate = matrix[row * 4] * vertex[0]
                    + matrix[row * 4 + 1] * vertex[1]
                    + matrix[row * 4 + 2] * vertex[2]
                    + matrix[row * 4 + 3];
                !coordinate.is_finite() || coordinate.abs() > MAX_ABS_MM
            })
        }) {
            return Err(SketchupSceneImportError::InvalidTransform);
        }
        let transform = Transform::from_matrix(matrix)
            .map_err(|_| SketchupSceneImportError::InvalidTransform)?;
        instances.push(ParsedInstance {
            definition: instance.definition,
            name: instance.name,
            transform,
            visible: instance.visible,
        });
    }

    definitions.sort_by(|left, right| left.source_id.cmp(&right.source_id));
    instances.sort_by(|left, right| {
        left.definition
            .cmp(&right.definition)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| {
                left.transform
                    .matrix()
                    .iter()
                    .map(|value| value.to_bits())
                    .cmp(right.transform.matrix().iter().map(|value| value.to_bits()))
            })
            .then_with(|| left.visible.cmp(&right.visible))
    });

    let mut diagnostics = vec![
        ImportDiagnostic::new(
            ImportDiagnosticSeverity::Info,
            "sketchup_components_and_transforms_preserved",
            None,
            instance_count as u32,
        )
        .map_err(|_| SketchupSceneImportError::InvalidGeometry)?,
    ];
    for (count, code) in [
        (
            file.metadata.material_assignments,
            "sketchup_material_assignments_dropped",
        ),
        (file.metadata.textures, "sketchup_textures_and_uvs_dropped"),
        (file.metadata.tags, "sketchup_tags_flattened"),
        (file.metadata.scenes, "sketchup_scenes_dropped"),
        (
            file.metadata.unsupported_entities,
            "sketchup_unsupported_entities_dropped",
        ),
    ] {
        if count > 0 {
            diagnostics.push(
                ImportDiagnostic::new(ImportDiagnosticSeverity::Warning, code, None, count)
                    .map_err(|_| SketchupSceneImportError::InvalidGeometry)?,
            );
        }
    }
    diagnostics.sort();

    Ok(ParsedSketchupScene {
        definitions,
        instances,
        diagnostics,
        triangle_count: total_triangles,
    })
}

pub fn plan_sketchup_scene_import(
    snapshot: &Snapshot,
    source: &[u8],
    source_name: &str,
) -> Result<CommandBatch, SketchupSceneImportError> {
    validate_text(source_name)?;
    if source_name.contains(['/', '\\']) {
        return Err(SketchupSceneImportError::InvalidSourceIdentity);
    }
    let scene = inspect_sketchup_scene(source)?;
    let import_id = snapshot
        .next_import_id()
        .map_err(|_| SketchupSceneImportError::IdSpaceExhausted)?;
    let definition_start = next_id(snapshot.definitions().map(|item| item.id().0))?;
    let feature_start = next_id(snapshot.features().map(|item| item.id().0))?;
    let occurrence_start = next_id(snapshot.occurrences().map(|item| item.id().0))?;

    let mut definition_ids = BTreeMap::new();
    let mut commands = Vec::new();
    let mut outputs = Vec::new();
    for (offset, definition) in scene.definitions.iter().enumerate() {
        let offset =
            u64::try_from(offset).map_err(|_| SketchupSceneImportError::IdSpaceExhausted)?;
        let definition_id = DefinitionId(
            definition_start
                .checked_add(offset)
                .ok_or(SketchupSceneImportError::IdSpaceExhausted)?,
        );
        let feature_id = FeatureId(
            feature_start
                .checked_add(offset)
                .ok_or(SketchupSceneImportError::IdSpaceExhausted)?,
        );
        definition_ids.insert(definition.source_id.clone(), definition_id);
        outputs.push(ImportOutputRef::Definition(definition_id));
        outputs.push(ImportOutputRef::Feature(feature_id));
        commands.push(CanonicalCommand::CreateDefinition {
            id: definition_id,
            name: definition.name.clone(),
        });
        commands.push(CanonicalCommand::CreateFeature {
            id: feature_id,
            definition_id,
            name: "SketchUp mesh".to_owned(),
            kind: FeatureKind::MeshBody(MeshBodySpec {
                schema: MESH_BODY_SCHEMA_V1.to_owned(),
                vertices_mm: definition.vertices_mm.clone(),
                triangles: definition.triangles.clone(),
                authority: MeshAuthority::ImportedSketchupScene { import_id },
            }),
        });
    }
    for (offset, instance) in scene.instances.iter().enumerate() {
        let offset =
            u64::try_from(offset).map_err(|_| SketchupSceneImportError::IdSpaceExhausted)?;
        let occurrence_id = OccurrenceId(
            occurrence_start
                .checked_add(offset)
                .ok_or(SketchupSceneImportError::IdSpaceExhausted)?,
        );
        outputs.push(ImportOutputRef::Occurrence(occurrence_id));
        commands.push(CanonicalCommand::CreateOccurrence {
            id: occurrence_id,
            definition_id: definition_ids[&instance.definition],
            name: instance.name.clone(),
            transform: instance.transform,
            parent: None,
            tag: None,
            visible: instance.visible,
        });
    }
    outputs.sort();
    let receipt = ImportReceipt::from_source_bytes(
        import_id,
        ImportFormat::SketchupScene,
        source,
        source_name,
        ImportUnitDecision::new(ImportLengthUnit::Inch, ImportUnitAuthority::FileDeclared),
        SKETCHUP_SCENE_PARSER_ID,
        SKETCHUP_SCENE_PARSER_VERSION,
        scene.diagnostics,
        outputs,
    )
    .map_err(|_| SketchupSceneImportError::InvalidSourceIdentity)?;
    commands.push(CanonicalCommand::RecordImport(receipt));
    Ok(CommandBatch::new(commands))
}

fn validate_text(value: &str) -> Result<(), SketchupSceneImportError> {
    if value.is_empty() || value.len() > MAX_TEXT_BYTES || value.chars().any(char::is_control) {
        Err(SketchupSceneImportError::InvalidText)
    } else {
        Ok(())
    }
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, SketchupSceneImportError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id != 0)
        .ok_or(SketchupSceneImportError::IdSpaceExhausted)
}
