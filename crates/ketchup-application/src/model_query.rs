//! Bounded projections of canonical snapshots; no geometry evaluation or parallel model.
//! Occurrences are document/root records (including group members), not expanded
//! definition-local instances. Definitions/features are the full canonical catalogs.
use ketchup_core::assembly::{AssemblyMateKind, AssemblyReferenceHealth};
use ketchup_core::document::{
    ClassificationCategoryId, ClassificationDimensionId, DefinitionId, FeatureId, FeatureKind,
    InstancePath, InstancePathStep, LocalGroupKey, LocalOccurrenceKey, OccurrenceId,
    SceneQueryBudgetExceeded, SceneQueryBudgetKind, Snapshot, TagId,
};
use ketchup_interaction::Vec3;
use ketchup_interaction::projection::{
    CanonicalInteractionProjection, InteractionProjection, ProjectedOccurrence,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, VecDeque, hash_map::RandomState};
use std::hash::BuildHasher;
use std::sync::Mutex;

pub const MAX_PAGE: usize = 100;
pub const MAX_OUTPUT_BYTES: usize = 32 * 1024;
pub const MAX_TEXT_BYTES: usize = 128;
pub const MAX_INSTANCE_INDEX_ITEMS: usize = 10_000;
pub const MAX_INSTANCE_PATH_STEPS: usize = 256;
pub const MAX_INSTANCE_INDEX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_WORKSET_ITEMS: usize = 10_000;
pub const MAX_WORKSET_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_ACTIVE_WORKSETS: usize = 16;
const MAX_CURSOR_BYTES: usize = 4096;
const PAGE_ITEM_BYTES: usize = 20 * 1024;
const MAX_PROPERTY_VALUES: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Occurrences,
    Instances,
    Definitions,
    Features,
    Relations,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PageRequest {
    pub kind: EntityKind,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Case-sensitive substring of the canonical name, or relation type for relation queries; no regex.
    #[serde(default)]
    pub search: String,
    #[serde(default)]
    pub definition_id: Option<u64>,
    /// Optional exact tag filter. Instance queries match any occurrence step in the path.
    #[serde(default)]
    pub tag_id: Option<u64>,
    /// Optional root-occurrence classification filter, supported for occurrences and instances.
    #[serde(default)]
    pub classification_dimension_id: Option<u64>,
    #[serde(default)]
    pub classification_category_id: Option<u64>,
    /// Optional inclusive world-space AABB; supported only for expanded instances.
    #[serde(default)]
    pub world_bounds_mm: Option<[[f64; 3]; 2]>,
    #[serde(default)]
    pub cursor: Option<String>,
}
fn default_limit() -> usize {
    50
}

fn valid_page_request(request: &PageRequest) -> bool {
    (1..=MAX_PAGE).contains(&request.limit)
        && request.search.len() <= 128
        && request.definition_id != Some(0)
        && request.tag_id != Some(0)
        && request.classification_dimension_id != Some(0)
        && request.classification_category_id != Some(0)
        && !(request.classification_category_id.is_some()
            && request.classification_dimension_id.is_none())
        && !(request.kind == EntityKind::Definitions && request.definition_id.is_some())
        && !((request.tag_id.is_some() || request.classification_dimension_id.is_some())
            && !matches!(
                request.kind,
                EntityKind::Occurrences | EntityKind::Instances
            ))
        && !(request.world_bounds_mm.is_some() && request.kind != EntityKind::Instances)
        && !request
            .world_bounds_mm
            .as_ref()
            .is_some_and(|bounds| !valid_world_bounds(bounds))
}

#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    InvalidInput,
    InvalidCursor,
    StaleCursor,
    CrossQueryCursor,
    OutputTooLarge,
    StaleWorkset,
    WorksetNotFound,
    NotFound,
}
impl QueryError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_params",
            Self::InvalidCursor => "invalid_cursor",
            Self::StaleCursor => "stale_cursor",
            Self::CrossQueryCursor => "cross_query_cursor",
            Self::OutputTooLarge => "output_too_large",
            Self::StaleWorkset => "stale_workset",
            Self::WorksetNotFound => "workset_not_found",
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WorksetToken {
    id: u64,
    generation: u64,
    document_id: u64,
    revision: u64,
    canonical_digest: String,
}

struct Workset {
    query: PageRequest,
    identities: Vec<Value>,
    source_total_matches: Option<u64>,
    complete: bool,
    incomplete_reason: Option<&'static str>,
    identity_text_bytes: usize,
}

#[derive(Default)]
struct WorksetStore {
    next_id: u64,
    order: VecDeque<u64>,
    items: BTreeMap<u64, Workset>,
}

/// A cursor scope belongs to one document host. Call `invalidate` after every
/// successful canonical mutation, Undo/Redo or document replacement, even when
/// revision/digest return to their previous values. Reads require only `&self`.
/// Tokens are host-local, keyed integrity checked, and do not survive restart.
struct InstanceIndex {
    identity: Value,
    generation: u64,
    state: InstanceIndexState,
    filter: Option<(InstanceFilter, Vec<usize>, Option<SpatialMetadata>)>,
}

enum InstanceIndexState {
    Ready(InteractionProjection),
    BudgetExceeded(SceneQueryBudgetExceeded),
}

#[derive(Clone, Debug, PartialEq)]
struct InstanceFilter {
    search: String,
    definition_id: Option<u64>,
    tag_id: Option<u64>,
    classification_dimension_id: Option<u64>,
    classification_category_id: Option<u64>,
    world_bounds_mm: Option<[[f64; 3]; 2]>,
}

#[derive(Clone, Debug)]
struct SpatialMetadata {
    query_bounds: [[f64; 3]; 2],
    indexed_instances: usize,
    bounds_tested: usize,
    candidates_before_metadata_filters: usize,
    unbounded_scope_instances: usize,
}

#[derive(Default)]
pub struct ModelQuery {
    key: RandomState,
    generation: u64,
    instances: Mutex<Option<InstanceIndex>>,
    worksets: Mutex<WorksetStore>,
}
impl ModelQuery {
    pub fn invalidate(&mut self) {
        *self.instances.get_mut().expect("instance index lock") = None;
        let worksets = self.worksets.get_mut().expect("workset lock");
        worksets.items.clear();
        worksets.order.clear();
        // Also rotate the key on wrap; no old token can become current again.
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.key = RandomState::new();
            self.generation = 0;
        }
    }

    fn refresh_instance_index<'a>(
        &self,
        snapshot: &Snapshot,
        cache: &'a mut Option<InstanceIndex>,
    ) -> &'a mut InstanceIndex {
        let current_identity = identity(snapshot);
        if cache.as_ref().is_none_or(|index| {
            index.identity != current_identity || index.generation != self.generation
        }) {
            let state = CanonicalInteractionProjection::from_snapshot_bounded(
                snapshot,
                MAX_INSTANCE_INDEX_ITEMS,
                MAX_INSTANCE_PATH_STEPS,
                MAX_INSTANCE_INDEX_TEXT_BYTES,
            )
            .map_or_else(
                InstanceIndexState::BudgetExceeded,
                InstanceIndexState::Ready,
            );
            *cache = Some(InstanceIndex {
                identity: current_identity,
                generation: self.generation,
                state,
                filter: None,
            });
        }
        cache.as_mut().expect("instance index initialized")
    }

    pub fn summary(&self, snapshot: &Snapshot) -> Value {
        let mut cache = self.instances.lock().expect("instance index lock");
        let index = self.refresh_instance_index(snapshot, &mut cache);
        let (instance_count, complete, resource_budget) = match &index.state {
            InstanceIndexState::Ready(projection) => (
                json!(projection.occurrences().len()),
                true,
                instance_budget_value(None, projection.occurrences().len()),
            ),
            InstanceIndexState::BudgetExceeded(exceeded) => (
                Value::Null,
                false,
                instance_budget_value(Some(*exceeded), 0),
            ),
        };
        json!({"identity":identity(snapshot),"coverage":coverage(None),
            "counts":{"root_occurrences":snapshot.occurrences().count(),
                "instances":instance_count,
                "definitions":snapshot.definitions().count(),"features":snapshot.features().count(),
                "relations":relation_count(snapshot)},
            "complete":complete,"resource_budget":resource_budget,
            "limits":{"max_page":MAX_PAGE,"max_output_bytes":MAX_OUTPUT_BYTES,
                "max_name_bytes":MAX_TEXT_BYTES,"max_search_bytes":128,
                "max_instance_index_items":MAX_INSTANCE_INDEX_ITEMS,
                "max_instance_path_steps":MAX_INSTANCE_PATH_STEPS,
                "max_instance_index_text_bytes":MAX_INSTANCE_INDEX_TEXT_BYTES}})
    }

    pub fn page(&self, snapshot: &Snapshot, request: &PageRequest) -> Result<Value, QueryError> {
        if !valid_page_request(request) {
            return Err(QueryError::InvalidInput);
        }
        let identity = identity(snapshot);
        let query = PageRequest {
            kind: request.kind,
            limit: request.limit,
            search: request.search.clone(),
            definition_id: request.definition_id,
            tag_id: request.tag_id,
            classification_dimension_id: request.classification_dimension_id,
            classification_category_id: request.classification_category_id,
            world_bounds_mm: request.world_bounds_mm,
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
        if request.kind == EntityKind::Instances {
            return self.instance_page(snapshot, request, identity, query, after);
        }
        if request.kind == EntityKind::Relations {
            return self.relation_page(snapshot, request, identity, query, after);
        }
        // Canonical maps iterate by ascending ID. Project only matching page rows,
        // never materialize the complete catalog or debug-format feature payloads.
        let rows: Box<dyn Iterator<Item = (u64, u64, &str)> + '_> = match request.kind {
            EntityKind::Instances => unreachable!("instances use the hierarchy projection"),
            EntityKind::Relations => {
                unreachable!("relations use the canonical relation projection")
            }
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
                || !entity_properties_match(snapshot, request.kind, id, request)
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
            let size = instance_item_size(&item)?;
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
            json!({"identity":identity,"coverage":coverage(Some(request.kind)),"items":items,"total_matches":total,
            "complete":!more,"next_cursor":next,"byte_limited":byte_limited}),
        )
    }

    fn instance_page(
        &self,
        snapshot: &Snapshot,
        request: &PageRequest,
        identity: Value,
        query: PageRequest,
        after: u64,
    ) -> Result<Value, QueryError> {
        let mut items = Vec::new();
        let mut bytes = 0;
        let mut last = after;
        let mut byte_limited = false;
        let mut cache = self.instances.lock().expect("instance index lock");
        let index = self.refresh_instance_index(snapshot, &mut cache);
        let InstanceIndex {
            state,
            filter: cached_filter,
            ..
        } = index;
        let projection = match state {
            InstanceIndexState::Ready(projection) => projection,
            InstanceIndexState::BudgetExceeded(exceeded) => {
                return Ok(instance_budget_page(identity, *exceeded));
            }
        };
        let filter = InstanceFilter {
            search: request.search.clone(),
            definition_id: request.definition_id,
            tag_id: request.tag_id,
            classification_dimension_id: request.classification_dimension_id,
            classification_category_id: request.classification_category_id,
            world_bounds_mm: request.world_bounds_mm,
        };
        if cached_filter
            .as_ref()
            .is_none_or(|(cached, _, _)| cached != &filter)
        {
            let occurrences = projection.occurrences();
            let (positions, spatial) = if let Some(bounds) = request.world_bounds_mm {
                let projected = projection.query_world_bounds(
                    Vec3::new(bounds[0][0], bounds[0][1], bounds[0][2]),
                    Vec3::new(bounds[1][0], bounds[1][1], bounds[1][2]),
                );
                let unbounded_scope_instances = if filter.search.is_empty()
                    && filter.definition_id.is_none()
                    && filter.tag_id.is_none()
                    && filter.classification_dimension_id.is_none()
                {
                    projected.unbounded_occurrences
                } else {
                    occurrences
                        .iter()
                        .filter(|occurrence| {
                            instance_metadata_matches(snapshot, occurrence, &filter)
                                && occurrence.box_proxy.is_none()
                        })
                        .count()
                };
                let positions = projected
                    .occurrence_indices
                    .iter()
                    .copied()
                    .filter(|position| {
                        instance_metadata_matches(snapshot, &occurrences[*position], &filter)
                    })
                    .collect();
                let spatial = SpatialMetadata {
                    query_bounds: bounds,
                    indexed_instances: projected.stats.indexed_items,
                    bounds_tested: projected.stats.bounds_tested,
                    candidates_before_metadata_filters: projected.stats.candidate_count,
                    unbounded_scope_instances,
                };
                (positions, Some(spatial))
            } else {
                (
                    occurrences
                        .iter()
                        .enumerate()
                        .filter(|(_, occurrence)| {
                            instance_metadata_matches(snapshot, occurrence, &filter)
                        })
                        .map(|(position, _)| position)
                        .collect(),
                    None,
                )
            };
            *cached_filter = Some((filter, positions, spatial));
        }
        let (_, positions, spatial) = cached_filter.as_ref().expect("instance filter initialized");
        let total = positions.len() as u64;
        let start = usize::try_from(after).map_err(|_| QueryError::InvalidCursor)?;
        if start > positions.len() {
            return Err(QueryError::InvalidCursor);
        }
        for position in positions.iter().skip(start).take(request.limit) {
            let item = instance_row(snapshot, &projection.occurrences()[*position])
                .ok_or(QueryError::NotFound)?;
            let size = instance_item_size(&item)?;
            if bytes + size > PAGE_ITEM_BYTES {
                byte_limited = true;
                break;
            }
            bytes += size;
            last += 1;
            items.push(item);
        }
        let more = last < total;
        let spatial_complete = spatial
            .as_ref()
            .is_none_or(|metadata| metadata.unbounded_scope_instances == 0);
        let next = more.then(|| {
            self.encode(&Cursor {
                identity: identity.clone(),
                generation: self.generation,
                query,
                after: last,
            })
        });
        Ok(
            json!({"identity":identity,"coverage":instance_coverage(spatial.as_ref()),"items":items,
            "total_matches":total,"total_matches_complete":spatial_complete,
            "complete":!more && spatial_complete,"page_complete":!more,"next_cursor":next,
            "byte_limited":byte_limited,
            "resource_budget":instance_budget_value(None, projection.occurrences().len()),
            "spatial_query":spatial.as_ref().map(spatial_metadata_value)}),
        )
    }

    fn relation_page(
        &self,
        snapshot: &Snapshot,
        request: &PageRequest,
        identity: Value,
        query: PageRequest,
        after: u64,
    ) -> Result<Value, QueryError> {
        let mut page = RelationPageState::new(after);
        for occurrence in snapshot.occurrences() {
            let source = json!({"kind":"occurrence","id":occurrence.id().0});
            page.consider(
                request,
                "uses_definition",
                &[occurrence.definition_id().0],
                || json!({"id":format!("occurrence:{}/definition", occurrence.id().0),
                    "relation_type":"uses_definition","direction":"outgoing",
                    "source":source,"target":{"kind":"definition","id":occurrence.definition_id().0},
                    "origin":{"kind":"canonical_occurrence","id":occurrence.id().0}}),
            )?;
            if let Some(parent) = occurrence.parent() {
                page.consider(
                    request,
                    "member_of_group",
                    &[occurrence.definition_id().0],
                    || {
                        json!({"id":format!("occurrence:{}/parent_group", occurrence.id().0),
                        "relation_type":"member_of_group","direction":"outgoing",
                        "source":{"kind":"occurrence","id":occurrence.id().0},
                        "target":{"kind":"group","id":parent.0},
                        "origin":{"kind":"canonical_occurrence","id":occurrence.id().0}})
                    },
                )?;
            }
        }
        for group in snapshot.groups() {
            if let Some(parent) = group.parent() {
                page.consider(request, "member_of_group", &[], || {
                    json!({"id":format!("group:{}/parent_group", group.id().0),
                        "relation_type":"member_of_group","direction":"outgoing",
                        "source":{"kind":"group","id":group.id().0},
                        "target":{"kind":"group","id":parent.0},
                        "origin":{"kind":"canonical_group","id":group.id().0}})
                })?;
            }
        }
        for occurrence in snapshot.local_occurrences() {
            let key = occurrence.key();
            page.consider(
                request,
                "uses_definition",
                &[key.definition_id.0, occurrence.definition_id().0],
                || json!({"id":format!("definition:{}/occurrence:{}/definition", key.definition_id.0, key.local_id.0),
                    "relation_type":"uses_definition","direction":"outgoing",
                    "source":{"kind":"local_occurrence","owner_definition_id":key.definition_id.0,
                        "local_id":key.local_id.0},
                    "target":{"kind":"definition","id":occurrence.definition_id().0},
                    "origin":{"kind":"canonical_local_occurrence","owner_definition_id":key.definition_id.0,
                        "local_id":key.local_id.0}}),
            )?;
            if let Some(parent) = occurrence.parent() {
                page.consider(
                    request,
                    "member_of_group",
                    &[key.definition_id.0, occurrence.definition_id().0],
                    || json!({"id":format!("definition:{}/occurrence:{}/parent_group", key.definition_id.0, key.local_id.0),
                        "relation_type":"member_of_group","direction":"outgoing",
                        "source":{"kind":"local_occurrence","owner_definition_id":key.definition_id.0,
                            "local_id":key.local_id.0},
                        "target":{"kind":"local_group","owner_definition_id":key.definition_id.0,
                            "local_id":parent.0},
                        "origin":{"kind":"canonical_local_occurrence","owner_definition_id":key.definition_id.0,
                            "local_id":key.local_id.0}}),
                )?;
            }
        }
        for group in snapshot.local_groups() {
            let key = group.key();
            if let Some(parent) = group.parent() {
                page.consider(
                    request,
                    "member_of_group",
                    &[key.definition_id.0],
                    || json!({"id":format!("definition:{}/group:{}/parent_group", key.definition_id.0, key.local_id.0),
                        "relation_type":"member_of_group","direction":"outgoing",
                        "source":{"kind":"local_group","owner_definition_id":key.definition_id.0,
                            "local_id":key.local_id.0},
                        "target":{"kind":"local_group","owner_definition_id":key.definition_id.0,
                            "local_id":parent.0},
                        "origin":{"kind":"canonical_local_group","owner_definition_id":key.definition_id.0,
                            "local_id":key.local_id.0}}),
                )?;
            }
        }
        for mate in snapshot.assembly_mates() {
            let a = mate.endpoint_a();
            let b = mate.endpoint_b();
            let definitions = [a, b]
                .into_iter()
                .filter_map(|endpoint| {
                    snapshot
                        .occurrence(endpoint.occurrence_id())
                        .map(|occurrence| occurrence.definition_id().0)
                })
                .collect::<Vec<_>>();
            page.consider(request, "assembly_mate", &definitions, || {
                json!({"id":format!("assembly_mate:{}", mate.id().0),
                    "relation_type":"assembly_mate","direction":"bidirectional",
                    "source":{"kind":"occurrence","id":a.occurrence_id().0,
                        "reference_health":assembly_health_value(a.health())},
                    "target":{"kind":"occurrence","id":b.occurrence_id().0,
                        "reference_health":assembly_health_value(b.health())},
                    "origin":{"kind":"canonical_assembly_mate","id":mate.id().0,
                        "schema":mate.schema()},"mate":assembly_kind_value(mate.kind())})
            })?;
        }
        page.finish(self, identity, query)
    }

    pub fn create_workset(
        &self,
        snapshot: &Snapshot,
        request: &PageRequest,
    ) -> Result<Value, QueryError> {
        if request.cursor.is_some() || !valid_page_request(request) {
            return Err(QueryError::InvalidInput);
        }
        let mut query = request.clone();
        query.limit = MAX_PAGE;
        query.cursor = None;
        let mut identities = Vec::new();
        let mut identity_text_bytes = 0usize;
        let mut source_total_matches = None;
        let mut complete = false;
        let mut incomplete_reason = None;
        loop {
            let page = self.page(snapshot, &query)?;
            source_total_matches = page["total_matches"].as_u64().or(source_total_matches);
            for item in page["items"].as_array().ok_or(QueryError::InvalidInput)? {
                let item_identity = item["id"].clone();
                let bytes = serde_json::to_vec(&item_identity)
                    .expect("query identity is JSON")
                    .len();
                if identities.len() == MAX_WORKSET_ITEMS {
                    incomplete_reason = Some("item_count");
                    break;
                }
                if identity_text_bytes.saturating_add(bytes) > MAX_WORKSET_TEXT_BYTES {
                    incomplete_reason = Some("identity_text_bytes");
                    break;
                }
                identity_text_bytes += bytes;
                identities.push(item_identity);
            }
            if incomplete_reason.is_some() {
                break;
            }
            if page["complete"] == true {
                complete = true;
                break;
            }
            let Some(cursor) = page["next_cursor"].as_str() else {
                incomplete_reason = Some("source_query_incomplete");
                break;
            };
            query.cursor = Some(cursor.to_owned());
        }
        query.cursor = None;
        let mut store = self.worksets.lock().expect("workset lock");
        store.next_id = store
            .next_id
            .checked_add(1)
            .ok_or(QueryError::OutputTooLarge)?;
        let id = store.next_id;
        if store.order.len() == MAX_ACTIVE_WORKSETS {
            let evicted = store
                .order
                .pop_front()
                .expect("bounded nonempty workset order");
            store.items.remove(&evicted);
        }
        store.order.push_back(id);
        store.items.insert(
            id,
            Workset {
                query,
                identities,
                source_total_matches,
                complete,
                incomplete_reason,
                identity_text_bytes,
            },
        );
        let token = self.encode_workset(snapshot, id);
        let workset = store.items.get(&id).expect("inserted workset");
        Ok(workset_value(snapshot, &token, workset))
    }

    pub fn workset_status(&self, snapshot: &Snapshot, handle: &str) -> Result<Value, QueryError> {
        let token = self.decode_workset(handle)?;
        if token.generation != self.generation
            || token.document_id != snapshot.document_id().0
            || token.revision != snapshot.revision_id()
            || token.canonical_digest != snapshot.canonical_digest()
        {
            return Err(QueryError::StaleWorkset);
        }
        let store = self.worksets.lock().expect("workset lock");
        let workset = store
            .items
            .get(&token.id)
            .ok_or(QueryError::WorksetNotFound)?;
        Ok(workset_value(snapshot, handle, workset))
    }

    /// Deliberately bounded metadata detail, not raw feature geometry or a state dump.
    pub fn detail(
        &self,
        snapshot: &Snapshot,
        kind: EntityKind,
        id: u64,
    ) -> Result<Value, QueryError> {
        if id == 0 || matches!(kind, EntityKind::Instances | EntityKind::Relations) {
            return Err(QueryError::InvalidInput);
        }
        let mut item = row(snapshot, kind, id).ok_or(QueryError::NotFound)?;
        let omitted = match kind {
            EntityKind::Instances | EntityKind::Relations => {
                unreachable!("instance/relation detail requires a qualified identity")
            }
            EntityKind::Occurrences => {
                let o = snapshot
                    .occurrence(OccurrenceId(id))
                    .ok_or(QueryError::NotFound)?;
                item["transform"] = json!(o.transform().matrix());
                item["color"] = json!(o.color());
                item["parent_group_id"] = json!(o.parent().map(|id| id.0));
                item["tag_id"] = json!(o.tag().map(|id| id.0));
                json!(["nested_hierarchy", "world_transform", "geometry"])
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
        let result = json!({"identity":identity(snapshot),"coverage":coverage(Some(kind)),"item":item,
            "completeness":{"metadata_only":true,"omitted":omitted}});
        (serde_json::to_vec(&result)
            .expect("bounded projection")
            .len()
            <= MAX_OUTPUT_BYTES)
            .then_some(result)
            .ok_or(QueryError::OutputTooLarge)
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

    fn encode_workset(&self, snapshot: &Snapshot, id: u64) -> String {
        let token = WorksetToken {
            id,
            generation: self.generation,
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
        };
        let payload = serde_json::to_string(&token).expect("workset token serialization");
        format!("{:016x}:{payload}", self.key.hash_one(&payload))
    }

    fn decode_workset(&self, handle: &str) -> Result<WorksetToken, QueryError> {
        if handle.len() > MAX_CURSOR_BYTES {
            return Err(QueryError::WorksetNotFound);
        }
        let (tag, payload) = handle.split_once(':').ok_or(QueryError::WorksetNotFound)?;
        if tag.len() != 16 || u64::from_str_radix(tag, 16).ok() != Some(self.key.hash_one(payload))
        {
            return Err(QueryError::WorksetNotFound);
        }
        serde_json::from_str(payload).map_err(|_| QueryError::WorksetNotFound)
    }
}

struct RelationPageState {
    after: u64,
    total: u64,
    last: u64,
    items: Vec<Value>,
    bytes: usize,
    byte_limited: bool,
}

impl RelationPageState {
    fn new(after: u64) -> Self {
        Self {
            after,
            total: 0,
            last: after,
            items: Vec::new(),
            bytes: 0,
            byte_limited: false,
        }
    }

    fn consider(
        &mut self,
        request: &PageRequest,
        relation_type: &str,
        definition_ids: &[u64],
        item: impl FnOnce() -> Value,
    ) -> Result<(), QueryError> {
        if !relation_type.contains(&request.search)
            || request
                .definition_id
                .is_some_and(|id| !definition_ids.contains(&id))
        {
            return Ok(());
        }
        self.total += 1;
        if self.total <= self.after || self.items.len() == request.limit || self.byte_limited {
            return Ok(());
        }
        let item = item();
        let size = instance_item_size(&item)?;
        if self.bytes + size > PAGE_ITEM_BYTES {
            self.byte_limited = true;
            return Ok(());
        }
        self.bytes += size;
        self.last = self.total;
        self.items.push(item);
        Ok(())
    }

    fn finish(
        self,
        query_owner: &ModelQuery,
        identity: Value,
        query: PageRequest,
    ) -> Result<Value, QueryError> {
        if self.after > self.total {
            return Err(QueryError::InvalidCursor);
        }
        let more = self.last < self.total;
        let next = more.then(|| {
            query_owner.encode(&Cursor {
                identity: identity.clone(),
                generation: query_owner.generation,
                query,
                after: self.last,
            })
        });
        Ok(
            json!({"identity":identity,"coverage":coverage(Some(EntityKind::Relations)),
            "items":self.items,"total_matches":self.total,"total_matches_complete":true,
            "complete":!more,"page_complete":!more,"next_cursor":next,
            "byte_limited":self.byte_limited,
            "resource_budget":{"status":"within_budget","resource":"streamed_canonical_relations",
                "max_page":MAX_PAGE,"max_output_bytes":MAX_OUTPUT_BYTES}}),
        )
    }
}

pub fn identity(snapshot: &Snapshot) -> Value {
    json!({"document_id":snapshot.document_id().0,"revision":snapshot.revision_id(),
        "canonical_digest":snapshot.canonical_digest()})
}

fn workset_value(snapshot: &Snapshot, handle: &str, workset: &Workset) -> Value {
    json!({"identity":identity(snapshot),"workset_handle":handle,
        "scope":{"source":"model_query","query":workset.query,
            "entity_kind":workset.query.kind},
        "item_count":workset.identities.len(),
        "source_total_matches":workset.source_total_matches,
        "identity_status":{"stale":false,"missing_identity_count":0},
        "completeness":{"complete":workset.complete,
            "usable_for_batch":workset.complete,
            "reason":workset.incomplete_reason},
        "resource_budget":{"status":if workset.complete {"within_budget"} else {"incomplete"},
            "max_items":MAX_WORKSET_ITEMS,"max_identity_text_bytes":MAX_WORKSET_TEXT_BYTES,
            "identity_text_bytes":workset.identity_text_bytes,
            "max_active_worksets":MAX_ACTIVE_WORKSETS}})
}

fn coverage(kind: Option<EntityKind>) -> Value {
    json!({"occurrences":"root_records_including_group_members",
        "instances":"expanded_qualified_hierarchy","definitions":"canonical_catalog",
        "features":"canonical_catalog",
        "relations":"canonical_hierarchy_definition_and_assembly_edges",
        "nested_hierarchy":kind == Some(EntityKind::Instances),
        "relations_streamed":kind == Some(EntityKind::Relations),
        "spatial":false,"geometry_evaluated":false})
}

fn relation_count(snapshot: &Snapshot) -> usize {
    snapshot.occurrences().count()
        + snapshot
            .occurrences()
            .filter(|occurrence| occurrence.parent().is_some())
            .count()
        + snapshot
            .groups()
            .filter(|group| group.parent().is_some())
            .count()
        + snapshot.local_occurrences().count()
        + snapshot
            .local_occurrences()
            .filter(|occurrence| occurrence.parent().is_some())
            .count()
        + snapshot
            .local_groups()
            .filter(|group| group.parent().is_some())
            .count()
        + snapshot.assembly_mates().count()
}

fn assembly_health_value(health: AssemblyReferenceHealth) -> Value {
    match health {
        AssemblyReferenceHealth::Resolved => json!({"status":"resolved"}),
        AssemblyReferenceHealth::Broken => json!({"status":"broken"}),
        AssemblyReferenceHealth::Ambiguous { candidate_count } => {
            json!({"status":"ambiguous","candidate_count":candidate_count})
        }
        AssemblyReferenceHealth::Lost => json!({"status":"lost"}),
    }
}

fn assembly_kind_value(kind: AssemblyMateKind) -> Value {
    match kind {
        AssemblyMateKind::CoincidentPlanar {
            offset_mm,
            reversed,
        } => json!({"kind":"coincident_planar","offset_mm":offset_mm,"reversed":reversed}),
        AssemblyMateKind::ConcentricAxial { reversed } => {
            json!({"kind":"concentric_axial","reversed":reversed})
        }
        AssemblyMateKind::Distance { distance_mm } => {
            json!({"kind":"distance","distance_mm":distance_mm})
        }
        AssemblyMateKind::Angle { angle_degrees } => {
            json!({"kind":"angle","angle_degrees":angle_degrees})
        }
    }
}
fn instance_budget_page(identity: Value, exceeded: SceneQueryBudgetExceeded) -> Value {
    json!({"identity":identity,"coverage":coverage(Some(EntityKind::Instances)),"items":[],
        "total_matches":Value::Null,"total_matches_complete":false,"complete":false,
        "page_complete":false,"next_cursor":Value::Null,"byte_limited":false,
        "resource_budget":instance_budget_value(Some(exceeded), 0),
        "spatial_query":Value::Null})
}

fn instance_budget_value(
    exceeded: Option<SceneQueryBudgetExceeded>,
    allocated_items: usize,
) -> Value {
    match exceeded {
        None => json!({"status":"within_budget","resource":"expanded_instance_index",
            "max_items":MAX_INSTANCE_INDEX_ITEMS,"max_path_steps":MAX_INSTANCE_PATH_STEPS,
            "max_text_bytes":MAX_INSTANCE_INDEX_TEXT_BYTES,"allocated_items":allocated_items}),
        Some(exceeded) => json!({"status":"exceeded","resource":"expanded_instance_index",
            "reason":match exceeded.kind {
                SceneQueryBudgetKind::Occurrences => "occurrence_count",
                SceneQueryBudgetKind::PathSteps => "path_steps",
                SceneQueryBudgetKind::TextBytes => "text_bytes",
            },"limit":exceeded.limit,"observed_at_least":exceeded.observed_at_least,
            "max_items":MAX_INSTANCE_INDEX_ITEMS,"max_path_steps":MAX_INSTANCE_PATH_STEPS,
            "max_text_bytes":MAX_INSTANCE_INDEX_TEXT_BYTES,"allocated_items":allocated_items}),
    }
}

fn instance_coverage(spatial: Option<&SpatialMetadata>) -> Value {
    let Some(spatial) = spatial else {
        return coverage(Some(EntityKind::Instances));
    };
    json!({"occurrences":"root_records_including_group_members",
        "instances":"expanded_qualified_hierarchy","definitions":"canonical_catalog",
        "features":"canonical_catalog",
        "relations":"canonical_hierarchy_definition_and_assembly_edges",
        "nested_hierarchy":true,"relations_streamed":false,"spatial":true,
        "spatial_candidates_complete":spatial.unbounded_scope_instances == 0,
        "geometry_evaluated":false})
}
fn spatial_metadata_value(spatial: &SpatialMetadata) -> Value {
    json!({"query_world_bounds_mm":spatial.query_bounds,"predicate":"intersects",
        "coordinate_space":"world_mm","bounds_origin":"canonical_profile_extrusion_proxy",
        "bounds_are_conservative":true,"geometry_evaluated":false,
        "indexed_instances":spatial.indexed_instances,"bounds_tested":spatial.bounds_tested,
        "candidates_before_metadata_filters":spatial.candidates_before_metadata_filters,
        "unbounded_scope_instances":spatial.unbounded_scope_instances,
        "candidates_complete":spatial.unbounded_scope_instances == 0})
}
fn valid_world_bounds(bounds: &[[f64; 3]; 2]) -> bool {
    bounds.iter().flatten().all(|value| value.is_finite())
        && (0..3).all(|axis| bounds[0][axis] <= bounds[1][axis])
}
fn instance_metadata_matches(
    snapshot: &Snapshot,
    occurrence: &ProjectedOccurrence,
    filter: &InstanceFilter,
) -> bool {
    (occurrence.occurrence_name.contains(&filter.search)
        || occurrence.definition_name.contains(&filter.search))
        && filter
            .definition_id
            .is_none_or(|id| id == occurrence.body.definition_id.0)
        && filter
            .tag_id
            .is_none_or(|id| instance_has_tag(snapshot, &occurrence.instance_path, TagId(id)))
        && classification_matches(
            snapshot,
            occurrence.instance_path.root_occurrence(),
            filter.classification_dimension_id,
            filter.classification_category_id,
        )
}
fn entity_properties_match(
    snapshot: &Snapshot,
    kind: EntityKind,
    id: u64,
    request: &PageRequest,
) -> bool {
    if kind != EntityKind::Occurrences {
        return true;
    }
    let Some(occurrence) = snapshot.occurrence(OccurrenceId(id)) else {
        return false;
    };
    request
        .tag_id
        .is_none_or(|tag_id| occurrence.tag() == Some(TagId(tag_id)))
        && classification_matches(
            snapshot,
            occurrence.id(),
            request.classification_dimension_id,
            request.classification_category_id,
        )
}

fn classification_matches(
    snapshot: &Snapshot,
    occurrence_id: OccurrenceId,
    dimension_id: Option<u64>,
    category_id: Option<u64>,
) -> bool {
    let Some(dimension_id) = dimension_id else {
        return true;
    };
    let assigned =
        snapshot.occurrence_classification(occurrence_id, ClassificationDimensionId(dimension_id));
    category_id.map_or_else(
        || assigned.is_some(),
        |id| assigned == Some(ClassificationCategoryId(id)),
    )
}

fn instance_has_tag(snapshot: &Snapshot, path: &InstancePath, wanted: TagId) -> bool {
    let Some(root) = snapshot.occurrence(path.root_occurrence()) else {
        return false;
    };
    if root.tag() == Some(wanted) {
        return true;
    }
    let mut owner_definition_id = root.definition_id();
    for step in path.steps() {
        if let InstancePathStep::Occurrence(local_id) = *step {
            let Some(local) = snapshot.local_occurrence(LocalOccurrenceKey {
                definition_id: owner_definition_id,
                local_id,
            }) else {
                return false;
            };
            if local.tag() == Some(wanted) {
                return true;
            }
            owner_definition_id = local.definition_id();
        }
    }
    false
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
fn instance_item_size(item: &Value) -> Result<usize, QueryError> {
    let size = serde_json::to_vec(item).expect("bounded projection").len() + 1;
    (size <= PAGE_ITEM_BYTES)
        .then_some(size)
        .ok_or(QueryError::OutputTooLarge)
}

fn qualified_path(
    snapshot: &Snapshot,
    path: &InstancePath,
    step_count: usize,
) -> Option<(Value, String, DefinitionId)> {
    let root = snapshot.occurrence(path.root_occurrence())?;
    let mut owner_definition_id = root.definition_id();
    let mut steps = Vec::new();
    let mut key = format!("root:{}", path.root_occurrence().0);
    for step in path.steps().iter().take(step_count) {
        match *step {
            InstancePathStep::Group(local_id) => {
                snapshot.local_group(LocalGroupKey {
                    definition_id: owner_definition_id,
                    local_id,
                })?;
                steps.push(json!({"owner_definition_id":owner_definition_id.0,
                    "kind":"group","local_id":local_id.0}));
                key.push_str(&format!(
                    "/definition:{}/group:{}",
                    owner_definition_id.0, local_id.0
                ));
            }
            InstancePathStep::Occurrence(local_id) => {
                let occurrence = snapshot.local_occurrence(LocalOccurrenceKey {
                    definition_id: owner_definition_id,
                    local_id,
                })?;
                steps.push(json!({"owner_definition_id":owner_definition_id.0,
                    "kind":"occurrence","local_id":local_id.0}));
                key.push_str(&format!(
                    "/definition:{}/occurrence:{}",
                    owner_definition_id.0, local_id.0
                ));
                owner_definition_id = occurrence.definition_id();
            }
        }
    }
    Some((
        json!({"root_occurrence_id":path.root_occurrence().0,"steps":steps}),
        key,
        owner_definition_id,
    ))
}

fn instance_row(snapshot: &Snapshot, occurrence: &ProjectedOccurrence) -> Option<Value> {
    let resolved = snapshot
        .resolve_instance_path(&occurrence.instance_path)
        .ok()?;
    let (path, id, final_definition_id) = qualified_path(
        snapshot,
        &occurrence.instance_path,
        occurrence.instance_path.steps().len(),
    )?;
    if resolved.definition_id != occurrence.body.definition_id
        || final_definition_id != occurrence.body.definition_id
        || resolved.world_transform != occurrence.canonical_world_transform
    {
        return None;
    }
    let parent_path = if occurrence.instance_path.is_root() {
        Value::Null
    } else {
        qualified_path(
            snapshot,
            &occurrence.instance_path,
            occurrence.instance_path.steps().len() - 1,
        )?
        .0
    };
    let instance_depth = occurrence
        .instance_path
        .steps()
        .iter()
        .filter(|step| matches!(step, InstancePathStep::Occurrence(_)))
        .count();
    Some(
        json!({"id":id,"instance_path":path,"parent_path":parent_path,
        "instance_depth":instance_depth,
        "path_step_count":occurrence.instance_path.steps().len(),
        "definition_id":occurrence.body.definition_id.0,
        "occurrence_name":bounded_text(&occurrence.occurrence_name),
        "definition_name":bounded_text(&occurrence.definition_name),
        "world_transform":resolved.world_transform.matrix(),
        "world_bounds":instance_bounds(occurrence),
        "properties":instance_properties(snapshot, &occurrence.instance_path)?,
        "visible":occurrence.visible,
        "shared_occurrence_count":occurrence.shared_occurrence_count}),
    )
}

fn instance_properties(snapshot: &Snapshot, path: &InstancePath) -> Option<Value> {
    let root = snapshot.occurrence(path.root_occurrence())?;
    let mut tags = Vec::new();
    if let Some(tag_id) = root.tag() {
        tags.push(tag_metadata(
            snapshot,
            tag_id,
            json!({"kind":"root_occurrence","occurrence_id":root.id().0}),
        )?);
    }
    let mut owner_definition_id = root.definition_id();
    for (path_step_index, step) in path.steps().iter().enumerate() {
        if let InstancePathStep::Occurrence(local_id) = *step {
            let local = snapshot.local_occurrence(LocalOccurrenceKey {
                definition_id: owner_definition_id,
                local_id,
            })?;
            if let Some(tag_id) = local.tag() {
                tags.push(tag_metadata(
                    snapshot,
                    tag_id,
                    json!({"kind":"local_occurrence","owner_definition_id":owner_definition_id.0,
                        "local_id":local_id.0,"path_step_index":path_step_index}),
                )?);
            }
            owner_definition_id = local.definition_id();
        }
    }
    Some(json!({"tags":tags,"tag_match_scope":"any_occurrence_step",
        "classifications":classification_metadata(snapshot, root.id()),
        "classification_scope":"root_occurrence"}))
}

fn occurrence_properties(snapshot: &Snapshot, occurrence_id: OccurrenceId) -> Option<Value> {
    let occurrence = snapshot.occurrence(occurrence_id)?;
    let tag = if let Some(tag_id) = occurrence.tag() {
        Some(tag_metadata(
            snapshot,
            tag_id,
            json!({"kind":"root_occurrence","occurrence_id":occurrence_id.0}),
        )?)
    } else {
        None
    };
    Some(
        json!({"tag":tag,"classifications":classification_metadata(snapshot, occurrence_id),
        "classification_scope":"root_occurrence"}),
    )
}

fn tag_metadata(snapshot: &Snapshot, tag_id: TagId, source: Value) -> Option<Value> {
    let tag = snapshot.tag(tag_id)?;
    Some(json!({"id":tag.id().0,"name":bounded_text(tag.name()),
        "visible":tag.visible(),"source":source}))
}

fn classification_metadata(snapshot: &Snapshot, occurrence_id: OccurrenceId) -> Value {
    let mut items = Vec::new();
    let mut total = 0;
    for (dimension_id, category_id) in snapshot.occurrence_classifications(occurrence_id) {
        total += 1;
        if items.len() == MAX_PROPERTY_VALUES {
            continue;
        }
        let Some(dimension) = snapshot.classification_dimension(dimension_id) else {
            continue;
        };
        let Some(category) = dimension.category(category_id) else {
            continue;
        };
        items.push(json!({"dimension_id":dimension_id.0,
            "dimension_name":bounded_text(dimension.name()),"category_id":category_id.0,
            "category_name":bounded_text(category.name())}));
    }
    json!({"items":items,"total":total,"complete":total <= MAX_PROPERTY_VALUES})
}

fn instance_bounds(occurrence: &ProjectedOccurrence) -> Value {
    let Some(bounds) = occurrence.box_proxy else {
        return Value::Null;
    };
    let max = bounds.origin_mm + bounds.size_mm;
    json!({"min_mm":[bounds.origin_mm.x,bounds.origin_mm.y,bounds.origin_mm.z],
        "max_mm":[max.x,max.y,max.z],"coordinate_space":"world_mm",
        "origin":{"kind":"canonical_profile_extrusion_proxy",
            "profile_feature_id":occurrence.body.profile_feature_id.map(|id|id.0),
            "extrusion_feature_id":occurrence.body.extrusion_feature_id.map(|id|id.0)},
        "conservative":true,"geometry_evaluated":false})
}

fn row(snapshot: &Snapshot, kind: EntityKind, id: u64) -> Option<Value> {
    Some(match kind {
        EntityKind::Instances | EntityKind::Relations => return None,
        EntityKind::Occurrences => {
            let o = snapshot.occurrence(OccurrenceId(id))?;
            json!({"id":id,"definition_id":o.definition_id().0,"name":bounded_text(o.name()),
                "visible":o.visible(),"grounded":snapshot.occurrence_is_grounded(o.id()),
                "properties":occurrence_properties(snapshot, o.id())?})
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_instance_item_fails_instead_of_issuing_a_non_progressing_cursor() {
        let item = json!({"instance_path":"x".repeat(PAGE_ITEM_BYTES)});
        assert_eq!(instance_item_size(&item), Err(QueryError::OutputTooLarge));
    }
}
