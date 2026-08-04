use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use crate::document::{
    CanonicalError, CanonicalNode, Definition, DefinitionId, Dimension, DocumentStore, Feature,
    FeatureId, FeatureKind, Group, GroupId, NodeId, Occurrence, OccurrenceId, ProductModel,
    Snapshot, TagId, Transform, UnitSystem,
};

const MAGIC: &[u8; 10] = b"KETCHUPDOC";
const PRODUCT_SCHEMA: u16 = 2;
const RESEARCH_SCHEMA: u16 = 1;
const LEGACY_SCHEMA: u16 = 0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationLoss {
    pub node_id: NodeId,
    pub field: &'static str,
    pub reason: &'static str,
}

pub struct LoadOutcome {
    pub document: DocumentStore,
    pub source_schema: u16,
    pub migration_losses: Vec<MigrationLoss>,
}

#[must_use]
pub fn save(snapshot: &Snapshot) -> Vec<u8> {
    let product = snapshot.product();
    let schema = if product.definitions.is_empty()
        && product.features.is_empty()
        && product.occurrences.is_empty()
        && product.groups.is_empty()
    {
        RESEARCH_SCHEMA
    } else {
        PRODUCT_SCHEMA
    };

    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    push_u16(&mut bytes, schema);
    push_u64(&mut bytes, snapshot.revision_id());
    push_u32(&mut bytes, snapshot.node_count() as u32);
    for node in snapshot.nodes().values() {
        push_u64(&mut bytes, node.id().0);
        push_string(&mut bytes, node.name());
        push_string(&mut bytes, node.dimension().source_token());
        push_u64(&mut bytes, node.dimension().millimetres().to_bits());
        push_u32(&mut bytes, node.dependencies().len() as u32);
        for dependency in node.dependencies() {
            push_u64(&mut bytes, dependency.0);
        }
    }

    if schema == PRODUCT_SCHEMA {
        push_u64(&mut bytes, product.document_id.0);
        push_u8(
            &mut bytes,
            match product.units {
                UnitSystem::Millimetres => 1,
            },
        );
        push_u32(&mut bytes, product.definitions.len() as u32);
        for definition in product.definitions.values() {
            push_u64(&mut bytes, definition.id().0);
            push_string(&mut bytes, definition.name());
            push_u32(&mut bytes, definition.feature_ids().len() as u32);
            for feature_id in definition.feature_ids() {
                push_u64(&mut bytes, feature_id.0);
            }
        }
        push_u32(&mut bytes, product.features.len() as u32);
        for feature in product.features.values() {
            push_u64(&mut bytes, feature.id().0);
            push_u64(&mut bytes, feature.definition_id().0);
            push_string(&mut bytes, feature.name());
            match feature.kind() {
                FeatureKind::Profile { points_mm } => {
                    push_u8(&mut bytes, 1);
                    push_u32(&mut bytes, points_mm.len() as u32);
                    for point in points_mm {
                        push_u64(&mut bytes, point[0].to_bits());
                        push_u64(&mut bytes, point[1].to_bits());
                    }
                }
                FeatureKind::Extrusion { profile, height } => {
                    push_u8(&mut bytes, 2);
                    push_u64(&mut bytes, profile.0);
                    push_string(&mut bytes, height.source_token());
                    push_u64(&mut bytes, height.millimetres().to_bits());
                }
            }
        }
        push_u32(&mut bytes, product.occurrences.len() as u32);
        for occurrence in product.occurrences.values() {
            push_u64(&mut bytes, occurrence.id().0);
            push_u64(&mut bytes, occurrence.definition_id().0);
            push_string(&mut bytes, occurrence.name());
            push_transform(&mut bytes, occurrence.transform());
            push_optional_id(&mut bytes, occurrence.parent().map(|id| id.0));
            push_optional_id(&mut bytes, occurrence.tag().map(|id| id.0));
            push_u8(&mut bytes, u8::from(occurrence.visible()));
        }
        push_u32(&mut bytes, product.groups.len() as u32);
        for group in product.groups.values() {
            push_u64(&mut bytes, group.id().0);
            push_string(&mut bytes, group.name());
            push_transform(&mut bytes, group.transform());
            push_optional_id(&mut bytes, group.parent().map(|id| id.0));
        }
    }
    bytes
}

pub fn save_atomic(
    path: impl AsRef<Path>,
    snapshot: &Snapshot,
) -> Result<(), FilePersistenceError> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&save(snapshot))?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| FilePersistenceError::Io(error.error))?;
    Ok(())
}

pub fn load_file(path: impl AsRef<Path>) -> Result<LoadOutcome, FilePersistenceError> {
    load(&fs::read(path)?).map_err(FilePersistenceError::Format)
}

pub fn load(bytes: &[u8]) -> Result<LoadOutcome, PersistenceError> {
    let mut reader = Reader::new(bytes);
    if reader.take(MAGIC.len())? != MAGIC {
        return Err(PersistenceError::InvalidMagic);
    }
    let schema = reader.u16()?;
    if !matches!(schema, LEGACY_SCHEMA | RESEARCH_SCHEMA | PRODUCT_SCHEMA) {
        return Err(PersistenceError::UnsupportedSchema(schema));
    }
    let revision_id = reader.u64()?;
    let node_count = reader.u32()?;
    let mut nodes = BTreeMap::new();
    let mut migration_losses = Vec::new();

    for _ in 0..node_count {
        let id = NodeId(reader.u64()?);
        let name = reader.string()?;
        let source_token = if schema == LEGACY_SCHEMA {
            String::new()
        } else {
            reader.string()?
        };
        let millimetres = f64::from_bits(reader.u64()?);
        let source_token = if schema == LEGACY_SCHEMA {
            migration_losses.push(MigrationLoss {
                node_id: id,
                field: "dimension.source_token",
                reason: "legacy schema stored only the canonical binary value",
            });
            format!("{millimetres:.17}")
        } else {
            source_token
        };
        let dependency_count = reader.u32()?;
        let mut dependencies = Vec::with_capacity(dependency_count as usize);
        for _ in 0..dependency_count {
            dependencies.push(NodeId(reader.u64()?));
        }
        let dimension = Dimension::new(source_token, millimetres)?;
        let node = CanonicalNode::new(id, name, dimension, dependencies)?;
        if nodes.insert(id, Arc::new(node)).is_some() {
            return Err(PersistenceError::DuplicateNode(id));
        }
    }

    let product = if schema == PRODUCT_SCHEMA {
        read_product(&mut reader)?
    } else {
        ProductModel::default()
    };
    if !reader.is_finished() {
        return Err(PersistenceError::TrailingBytes);
    }
    let document = DocumentStore::from_parts(revision_id, nodes, product)?;
    Ok(LoadOutcome {
        document,
        source_schema: schema,
        migration_losses,
    })
}

fn read_product(reader: &mut Reader<'_>) -> Result<ProductModel, PersistenceError> {
    let mut product = ProductModel {
        document_id: crate::document::DocumentId(reader.u64()?),
        units: match reader.u8()? {
            1 => UnitSystem::Millimetres,
            units => return Err(PersistenceError::UnsupportedUnits(units)),
        },
        ..ProductModel::default()
    };
    for _ in 0..reader.u32()? {
        let id = DefinitionId(reader.u64()?);
        let name = reader.string()?;
        let mut feature_ids = Vec::new();
        for _ in 0..reader.u32()? {
            feature_ids.push(FeatureId(reader.u64()?));
        }
        if product
            .definitions
            .insert(
                id,
                Arc::new(Definition {
                    id,
                    name,
                    feature_ids,
                }),
            )
            .is_some()
        {
            return Err(PersistenceError::DuplicateDefinition(id));
        }
    }
    for _ in 0..reader.u32()? {
        let id = FeatureId(reader.u64()?);
        let definition_id = DefinitionId(reader.u64()?);
        let name = reader.string()?;
        let kind = match reader.u8()? {
            1 => {
                let mut points_mm = Vec::new();
                for _ in 0..reader.u32()? {
                    points_mm.push([f64::from_bits(reader.u64()?), f64::from_bits(reader.u64()?)]);
                }
                FeatureKind::Profile { points_mm }
            }
            2 => FeatureKind::Extrusion {
                profile: FeatureId(reader.u64()?),
                height: Dimension::new(reader.string()?, f64::from_bits(reader.u64()?))?,
            },
            kind => return Err(PersistenceError::InvalidFeatureKind(kind)),
        };
        if product
            .features
            .insert(
                id,
                Arc::new(Feature {
                    id,
                    definition_id,
                    name,
                    kind,
                }),
            )
            .is_some()
        {
            return Err(PersistenceError::DuplicateFeature(id));
        }
    }
    for _ in 0..reader.u32()? {
        let id = OccurrenceId(reader.u64()?);
        let occurrence = Occurrence {
            id,
            definition_id: DefinitionId(reader.u64()?),
            name: reader.string()?,
            transform: reader.transform()?,
            parent: reader.optional_id()?.map(GroupId),
            tag: reader.optional_id()?.map(TagId),
            visible: reader.boolean()?,
        };
        if product
            .occurrences
            .insert(id, Arc::new(occurrence))
            .is_some()
        {
            return Err(PersistenceError::DuplicateOccurrence(id));
        }
    }
    for _ in 0..reader.u32()? {
        let id = GroupId(reader.u64()?);
        let group = Group {
            id,
            name: reader.string()?,
            transform: reader.transform()?,
            parent: reader.optional_id()?.map(GroupId),
        };
        if product.groups.insert(id, Arc::new(group)).is_some() {
            return Err(PersistenceError::DuplicateGroup(id));
        }
    }
    Ok(product)
}

#[derive(Debug)]
pub enum FilePersistenceError {
    Io(io::Error),
    Format(PersistenceError),
}

impl fmt::Display for FilePersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::Format(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FilePersistenceError {}

impl From<io::Error> for FilePersistenceError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PersistenceError {
    Truncated,
    InvalidMagic,
    UnsupportedSchema(u16),
    InvalidUtf8,
    LengthOverflow,
    TrailingBytes,
    InvalidBoolean(u8),
    UnsupportedUnits(u8),
    InvalidFeatureKind(u8),
    DuplicateNode(NodeId),
    DuplicateDefinition(DefinitionId),
    DuplicateFeature(FeatureId),
    DuplicateOccurrence(OccurrenceId),
    DuplicateGroup(GroupId),
    InvalidCanonicalData(CanonicalError),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("document is truncated"),
            Self::InvalidMagic => formatter.write_str("document magic is invalid"),
            Self::UnsupportedSchema(schema) => {
                write!(formatter, "document schema {schema} is unsupported")
            }
            Self::InvalidUtf8 => formatter.write_str("document string is not UTF-8"),
            Self::LengthOverflow => formatter.write_str("document length exceeds this platform"),
            Self::TrailingBytes => formatter.write_str("document has trailing bytes"),
            Self::InvalidBoolean(value) => write!(formatter, "document boolean {value} is invalid"),
            Self::UnsupportedUnits(units) => {
                write!(formatter, "document unit system {units} is unsupported")
            }
            Self::InvalidFeatureKind(kind) => write!(formatter, "feature kind {kind} is invalid"),
            Self::DuplicateNode(id) => write!(formatter, "document repeats node {}", id.0),
            Self::DuplicateDefinition(id) => {
                write!(formatter, "document repeats definition {}", id.0)
            }
            Self::DuplicateFeature(id) => write!(formatter, "document repeats feature {}", id.0),
            Self::DuplicateOccurrence(id) => {
                write!(formatter, "document repeats occurrence {}", id.0)
            }
            Self::DuplicateGroup(id) => write!(formatter, "document repeats group {}", id.0),
            Self::InvalidCanonicalData(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PersistenceError {}

impl From<CanonicalError> for PersistenceError {
    fn from(error: CanonicalError) -> Self {
        Self::InvalidCanonicalData(error)
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], PersistenceError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(PersistenceError::LengthOverflow)?;
        let value = self
            .bytes
            .get(self.cursor..end)
            .ok_or(PersistenceError::Truncated)?;
        self.cursor = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, PersistenceError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PersistenceError> {
        let bytes: [u8; 2] = self
            .take(2)?
            .try_into()
            .map_err(|_| PersistenceError::Truncated)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32, PersistenceError> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| PersistenceError::Truncated)?;
        Ok(u32::from_le_bytes(bytes))
    }

    fn u64(&mut self) -> Result<u64, PersistenceError> {
        let bytes: [u8; 8] = self
            .take(8)?
            .try_into()
            .map_err(|_| PersistenceError::Truncated)?;
        Ok(u64::from_le_bytes(bytes))
    }

    fn string(&mut self) -> Result<String, PersistenceError> {
        let length = usize::try_from(self.u32()?).map_err(|_| PersistenceError::LengthOverflow)?;
        let bytes = self.take(length)?;
        let value = std::str::from_utf8(bytes).map_err(|_| PersistenceError::InvalidUtf8)?;
        Ok(value.to_owned())
    }

    fn optional_id(&mut self) -> Result<Option<u64>, PersistenceError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u64()?)),
            value => Err(PersistenceError::InvalidBoolean(value)),
        }
    }

    fn boolean(&mut self) -> Result<bool, PersistenceError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(PersistenceError::InvalidBoolean(value)),
        }
    }

    fn transform(&mut self) -> Result<Transform, PersistenceError> {
        let mut matrix = [0.0; 16];
        for value in &mut matrix {
            *value = f64::from_bits(self.u64()?);
        }
        Ok(Transform::from_matrix(matrix)?)
    }

    const fn is_finished(&self) -> bool {
        self.cursor == self.bytes.len()
    }
}

fn push_u8(bytes: &mut Vec<u8>, value: u8) {
    bytes.push(value);
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    push_u32(bytes, value.len() as u32);
    bytes.extend_from_slice(value.as_bytes());
}

fn push_transform(bytes: &mut Vec<u8>, transform: Transform) {
    for value in transform.matrix() {
        push_u64(bytes, value.to_bits());
    }
}

fn push_optional_id(bytes: &mut Vec<u8>, id: Option<u64>) {
    match id {
        Some(id) => {
            push_u8(bytes, 1);
            push_u64(bytes, id);
        }
        None => push_u8(bytes, 0),
    }
}
