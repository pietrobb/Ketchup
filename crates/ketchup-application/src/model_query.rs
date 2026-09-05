//! Bounded projections of canonical snapshots; no geometry evaluation or parallel model.
//! Occurrences are document/root records (including group members), not expanded
//! definition-local instances. Definitions/features are the full canonical catalogs.
use ketchup_core::document::{DefinitionId, FeatureId, FeatureKind, OccurrenceId, Snapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;

pub const MAX_PAGE: usize = 100;
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;
pub const MAX_TEXT_BYTES: usize = 128;
const MAX_CURSOR_BYTES: usize = 4096;
const PAGE_ITEM_BYTES: usize = 20 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Occurrences,
    Definitions,
    Features,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub kind: EntityKind,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Case-sensitive substring of the complete canonical name; no regex.
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub definition_id: Option<u64>,
    #[serde(default)]
    pub cursor: Option<String>,
}
fn default_limit() -> usize {
    50
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    InvalidInput,
    InvalidCursor,
    StaleCursor,
    CrossQueryCursor,
    NotFound,
}
impl QueryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_params",
            Self::InvalidCursor => "invalid_cursor",
            Self::StaleCursor => "stale_cursor",
            Self::CrossQueryCursor => "cross_query_cursor",
            Self::NotFound => "entity_not_found",
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    identity: Value,
    generation: u64,
    query: PageRequest,
    after: u64,
}

/// A cursor scope belongs to one document host. Call `invalidate` after every
/// successful canonical mutation, Undo/Redo or document replacement, even when
/// revision/digest return to their previous values. Reads require only `&self`.
/// Tokens are host-local, keyed integrity checked, and do not survive restart.
#[derive(Default)]
pub struct ModelQuery {
    key: RandomState,
    generation: u64,
}
impl ModelQuery {
    pub fn invalidate(&mut self) {
        // Also rotate the key on wrap; no old token can become current again.
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.key = RandomState::new();
            self.generation = 0;
        }
    }

    pub fn summary(&self, snapshot: &Snapshot) -> Value {
        json!({"identity":identity(snapshot),"coverage":coverage(),
            "counts":{"root_occurrences":snapshot.occurrences().count(),
                "definitions":snapshot.definitions().count(),"features":snapshot.features().count()},
            "complete":true,"limits":{"max_page":MAX_PAGE,"max_output_bytes":MAX_OUTPUT_BYTES,
                "max_name_bytes":MAX_TEXT_BYTES,"max_search_bytes":128}})
    }

    pub fn page(&self, snapshot: &Snapshot, request: &PageRequest) -> Result<Value, QueryError> {
        if !(1..=MAX_PAGE).contains(&request.limit)
            || request.search.len() > 128
            || request.definition_id == Some(0)
            || (request.kind == EntityKind::Definitions && request.definition_id.is_some())
        {
            return Err(QueryError::InvalidInput);
        }
        let identity = identity(snapshot);
        let query = PageRequest {
            kind: request.kind,
            limit: request.limit,
            search: request.search.clone(),
            definition_id: request.definition_id,
            cursor: None,
        };
        let after = if let Some(token) = &request.cursor {
            let cursor = self.decode(token)?;
            if cursor.identity != identity || cursor.generation != self.generation {
                return Err(QueryError::StaleCursor);
            }
            if cursor.query != query {
                return Err(QueryError::CrossQueryCursor);
            }
            cursor.after
        } else {
            0
        };
        // Canonical maps iterate by ascending ID. Project only matching page rows,
        // never materialize the complete catalog or debug-format feature payloads.
        let rows: Box<dyn Iterator<Item = (u64, u64, &str)> + '_> = match request.kind {
            EntityKind::Occurrences => Box::new(
                snapshot
                    .occurrences()
                    .map(|o| (o.id().0, o.definition_id().0, o.name())),
            ),
            EntityKind::Definitions => Box::new(
                snapshot
                    .definitions()
                    .map(|d| (d.id().0, d.id().0, d.name())),
            ),
            EntityKind::Features => Box::new(
                snapshot
                    .features()
                    .map(|f| (f.id().0, f.definition_id().0, f.name())),
            ),
        };
        let mut items = Vec::new();
        let mut total = 0;
        let mut bytes = 0;
        let mut last = after;
        let mut more = false;
        let mut byte_limited = false;
        for (id, definition, name) in rows {
            if !name.contains(&request.search)
                || request.definition_id.is_some_and(|d| d != definition)
            {
                continue;
            }
            total += 1;
            if id <= after {
                continue;
            }
            if items.len() == request.limit || more {
                more = true;
                continue;
            }
            let item = row(snapshot, request.kind, id).ok_or(QueryError::NotFound)?;
            let size = serde_json::to_vec(&item).expect("bounded projection").len() + 1;
            if bytes + size > PAGE_ITEM_BYTES {
                more = true;
                byte_limited = true;
                continue;
            }
            bytes += size;
            last = id;
            items.push(item);
        }
        let next = more.then(|| {
            self.encode(&Cursor {
                identity: identity.clone(),
                generation: self.generation,
                query,
                after: last,
            })
        });
        Ok(
            json!({"identity":identity,"coverage":coverage(),"items":items,"total_matches":total,
            "complete":!more,"next_cursor":next,"byte_limited":byte_limited}),
        )
    }

    /// Deliberately bounded metadata detail, not raw feature geometry or a state dump.
    pub fn detail(
        &self,
        snapshot: &Snapshot,
        kind: EntityKind,
        id: u64,
    ) -> Result<Value, QueryError> {
        if id == 0 {
            return Err(QueryError::InvalidInput);
        }
        let mut item = row(snapshot, kind, id).ok_or(QueryError::NotFound)?;
        let omitted = match kind {
            EntityKind::Occurrences => {
                let o = snapshot
                    .occurrence(OccurrenceId(id))
                    .ok_or(QueryError::NotFound)?;
                item["transform"] = json!(o.transform().matrix());
                item["color"] = json!(o.color());
                item["parent_group_id"] = json!(o.parent().map(|id| id.0));
                item["tag_id"] = json!(o.tag().map(|id| id.0));
                json!([
                    "nested_hierarchy",
                    "world_transform",
                    "geometry",
                    "properties"
                ])
            }
            EntityKind::Definitions => {
                let d = snapshot
                    .definition(DefinitionId(id))
                    .ok_or(QueryError::NotFound)?;
                item["feature_ids"] = bounded_ids(d.feature_ids().iter().map(|id| id.0));
                item["body_count"] = json!(d.bodies().count());
                item["local_occurrence_count"] = json!(d.local_occurrence_ids().len());
                json!(["bodies", "nested_hierarchy", "properties"])
            }
            EntityKind::Features => json!(["parameters", "dependencies", "geometry", "properties"]),
        };
        Ok(
            json!({"identity":identity(snapshot),"coverage":coverage(),"item":item,
            "completeness":{"metadata_only":true,"omitted":omitted}}),
        )
    }

    fn encode(&self, cursor: &Cursor) -> String {
        let payload = serde_json::to_string(cursor).expect("cursor serialization");
        format!("{:016x}:{payload}", self.key.hash_one(&payload))
    }
    fn decode(&self, token: &str) -> Result<Cursor, QueryError> {
        if token.len() > MAX_CURSOR_BYTES {
            return Err(QueryError::InvalidCursor);
        }
        let (tag, payload) = token.split_once(':').ok_or(QueryError::InvalidCursor)?;
        if tag.len() != 16 || u64::from_str_radix(tag, 16).ok() != Some(self.key.hash_one(payload))
        {
            return Err(QueryError::InvalidCursor);
        }
        serde_json::from_str(payload).map_err(|_| QueryError::InvalidCursor)
    }
}

pub fn identity(snapshot: &Snapshot) -> Value {
    json!({"document_id":snapshot.document_id().0,"revision":snapshot.revision_id(),
        "canonical_digest":snapshot.canonical_digest()})
}
fn coverage() -> Value {
    json!({"occurrences":"root_records_including_group_members","definitions":"canonical_catalog",
        "features":"canonical_catalog","nested_hierarchy":false,"spatial":false,"geometry_evaluated":false})
}
pub fn bounded_text(text: &str) -> Value {
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    json!({"text":&text[..end],"original_bytes":text.len(),"truncated":end < text.len()})
}
pub fn bounded_ids(ids: impl Iterator<Item = u64>) -> Value {
    let mut items = Vec::new();
    let mut total = 0;
    for id in ids {
        total += 1;
        if items.len() < MAX_PAGE {
            items.push(id);
        }
    }
    json!({"ids":items,"total":total,"complete":total <= MAX_PAGE})
}
pub fn created_receipt(before: &Snapshot, after: &Snapshot) -> Value {
    json!({"definition_ids":bounded_ids(after.definitions().filter(|d|before.definition(d.id()).is_none()).map(|d|d.id().0)),
        "occurrence_ids":bounded_ids(after.occurrences().filter(|o|before.occurrence(o.id()).is_none()).map(|o|o.id().0)),
        "feature_ids":bounded_ids(after.features().filter(|f|before.feature(f.id()).is_none()).map(|f|f.id().0))})
}
fn row(snapshot: &Snapshot, kind: EntityKind, id: u64) -> Option<Value> {
    Some(match kind {
        EntityKind::Occurrences => {
            let o = snapshot.occurrence(OccurrenceId(id))?;
            json!({"id":id,"definition_id":o.definition_id().0,"name":bounded_text(o.name()),
                "visible":o.visible(),"grounded":snapshot.occurrence_is_grounded(o.id())})
        }
        EntityKind::Definitions => {
            let d = snapshot.definition(DefinitionId(id))?;
            json!({"id":id,"name":bounded_text(d.name()),"feature_count":d.feature_ids().len()})
        }
        EntityKind::Features => {
            let f = snapshot.feature(FeatureId(id))?;
            json!({"id":id,"definition_id":f.definition_id().0,"name":bounded_text(f.name()),
                "kind":feature_kind(f.kind()),"suppressed":snapshot.feature_is_suppressed(f.id())})
        }
    })
}
fn feature_kind(kind: &FeatureKind) -> &'static str {
    // Exhaustive matching never allocates or formats potentially huge payloads.
    match kind {
        FeatureKind::Workplane(_) => "Workplane",
        FeatureKind::Sketch(_) => "Sketch",
        FeatureKind::Profile { .. } => "Profile",
        FeatureKind::SegmentProfile { .. } => "SegmentProfile",
        FeatureKind::SpatialPath { .. } => "SpatialPath",
        FeatureKind::SplineProfile { .. } => "SplineProfile",
        FeatureKind::Extrusion { .. } => "Extrusion",
        FeatureKind::Pad(_) => "Pad",
        FeatureKind::SketchPocket(_) => "SketchPocket",
        FeatureKind::BottleProfileControl { .. } => "BottleProfileControl",
        FeatureKind::Revolve { .. } => "Revolve",
        FeatureKind::Shell { .. } => "Shell",
        FeatureKind::BottleEdgeFinish { .. } => "BottleEdgeFinish",
        FeatureKind::TopologyShell { .. } => "TopologyShell",
        FeatureKind::TopologyEdgeFinish { .. } => "TopologyEdgeFinish",
        FeatureKind::TopologyFaceOffset { .. } => "TopologyFaceOffset",
        FeatureKind::ThroughCut { .. } => "ThroughCut",
        FeatureKind::Pocket { .. } => "Pocket",
        FeatureKind::Boolean { .. } => "Boolean",
        FeatureKind::PlanarOffset { .. } => "PlanarOffset",
        FeatureKind::Sweep { .. } => "Sweep",
        FeatureKind::Loft { .. } => "Loft",
        FeatureKind::ImportedExactBody(_) => "ImportedExactBody",
        FeatureKind::RigidTransform { .. } => "RigidTransform",
        FeatureKind::MeshBody(_) => "MeshBody",
    }
}
