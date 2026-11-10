//! Tags (layers) that the reader which wrote a GLB knows for its nodes, such as a SketchUp
//! file's tags: glTF has no place for them, so they come beside the GLB. A part imports with
//! the tags of its node and of every node above it, so hiding a tag hides what the source
//! program hid with it.

use std::collections::{BTreeMap, BTreeSet};

use super::{GlbImportError, ParsedNode, validate_text};
use crate::document::{CanonicalCommand, Snapshot, TagId};

/// At most this many tags come with one GLB.
pub const MAX_GLB_TAGS: usize = 256;

/// A tag with its visibility in the source file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlbTag {
    pub name: String,
    pub visible: bool,
}

/// The tags of a GLB's nodes: `node_tags[n]` is the index in `tags` of glTF node `n`'s own
/// tag, if it has one. Empty `node_tags` tags nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GlbNodeTags {
    pub tags: Vec<GlbTag>,
    pub node_tags: Vec<Option<usize>>,
}

/// The commands that create the tags no tag of the document already names, and the tags of
/// each node's parts, by glTF node index.
pub(super) struct PlannedTags {
    pub(super) commands: Vec<CanonicalCommand>,
    pub(super) by_node: Vec<BTreeSet<TagId>>,
}

pub(super) fn plan_tags(
    snapshot: &Snapshot,
    tags: &GlbNodeTags,
    nodes: &[ParsedNode],
    traversal: &[usize],
) -> Result<PlannedTags, GlbImportError> {
    let mut planned = PlannedTags {
        commands: Vec::new(),
        by_node: vec![BTreeSet::new(); nodes.len()],
    };
    if tags.node_tags.is_empty() {
        return Ok(planned);
    }
    if tags.tags.len() > MAX_GLB_TAGS {
        return Err(GlbImportError::TooManyOutputs);
    }
    if tags.node_tags.len() != nodes.len()
        || tags
            .node_tags
            .iter()
            .flatten()
            .any(|tag| *tag >= tags.tags.len())
    {
        return Err(GlbImportError::InvalidReference);
    }
    for tag in &tags.tags {
        validate_text(&tag.name)?;
        if tag.name.trim().is_empty() {
            return Err(GlbImportError::InvalidText);
        }
    }

    let mut existing: BTreeMap<&str, TagId> = BTreeMap::new();
    for tag in snapshot.tags() {
        existing.entry(tag.name()).or_insert(tag.id());
    }
    let mut next = snapshot
        .tags()
        .map(|tag| tag.id().0)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(GlbImportError::IdSpaceExhausted)?;
    let mut ids: Vec<Option<TagId>> = vec![None; tags.tags.len()];
    let mut parents = vec![None; nodes.len()];
    for node in traversal {
        for child in &nodes[*node].children {
            parents[*child] = Some(*node);
        }
    }
    for node in traversal {
        // A root carries no tags from above.
        let mut carried = match parents[*node] {
            Some(parent) => planned.by_node[parent].clone(),
            None => BTreeSet::new(),
        };
        if let Some(index) = tags.node_tags[*node] {
            let tag = &tags.tags[index];
            let id = match ids[index] {
                Some(id) => id,
                None => {
                    let id = if let Some(id) = existing.get(tag.name.as_str()) {
                        *id
                    } else {
                        let id = TagId(next);
                        next = next
                            .checked_add(1)
                            .ok_or(GlbImportError::IdSpaceExhausted)?;
                        planned.commands.push(CanonicalCommand::CreateTag {
                            id,
                            name: tag.name.clone(),
                            visible: tag.visible,
                        });
                        id
                    };
                    // Two tags of one name are one tag, as they are in the document.
                    for (other, same) in tags.tags.iter().zip(ids.iter_mut()) {
                        if other.name == tag.name {
                            *same = Some(id);
                        }
                    }
                    id
                }
            };
            carried.insert(id);
        }
        planned.by_node[*node] = carried;
    }
    Ok(planned)
}

#[cfg(test)]
mod tests {
    use super::super::{inspect_glb, plan_glb_import_tagged};
    use super::*;
    use crate::document::{CommandBatch, DocumentStore, GroupId};

    /// The committed Blender GLB with its second and third root nested: the third under
    /// the second, the second under the first.
    fn garden() -> Vec<u8> {
        let (mut json, binary) = super::super::json_tests::sample();
        let roots: Vec<u64> = json["scenes"][0]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node.as_u64().unwrap())
            .collect();
        let [first, second, third] = [roots[0], roots[1], roots[2]];
        let nest = |json: &mut serde_json::Value, parent: u64, child: u64| {
            let node = &mut json["nodes"][usize::try_from(parent).unwrap()];
            if node.get("children").is_none() {
                node["children"] = serde_json::json!([]);
            }
            node["children"].as_array_mut().unwrap().push(child.into());
        };
        nest(&mut json, first, second);
        nest(&mut json, second, third);
        json["scenes"][0]["nodes"]
            .as_array_mut()
            .unwrap()
            .retain(|node| ![second, third].contains(&node.as_u64().unwrap()));
        super::super::json_tests::packed(&json, &binary)
    }

    /// The tags of every part a batch creates, by the glTF node its group stands for.
    fn tags_by_node(
        batch: &CommandBatch,
        traversal: &[usize],
    ) -> BTreeMap<usize, Vec<BTreeSet<TagId>>> {
        let groups: Vec<GroupId> = batch
            .commands()
            .iter()
            .filter_map(|command| match command {
                CanonicalCommand::CreateGroup { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let mut by_node = BTreeMap::<usize, Vec<BTreeSet<TagId>>>::new();
        for command in batch.commands() {
            if let CanonicalCommand::CreateOccurrence {
                parent: Some(parent),
                tags,
                ..
            } = command
            {
                let offset = groups.iter().position(|group| group == parent).unwrap();
                by_node
                    .entry(traversal[offset])
                    .or_default()
                    .push(tags.clone());
            }
        }
        by_node
    }

    /// A SketchUp group tagged "Roof" inside a component tagged "Walls": hiding either tag
    /// hides the roof's parts, as in SketchUp. Before, every tag of the file was flattened
    /// and the parts imported untagged.
    #[test]
    fn parts_import_with_the_tags_of_their_node_and_every_node_above() {
        let source = garden();
        let scene = inspect_glb(&source).unwrap();
        // The first root holding a node with parts below it; that node is the roof.
        let below = |root: usize| {
            let mut found = Vec::new();
            let mut stack = scene.nodes[root].children.clone();
            while let Some(node) = stack.pop() {
                found.push(node);
                stack.extend(&scene.nodes[node].children);
            }
            found
        };
        let (root, inner) = scene
            .traversal
            .iter()
            .find_map(|root| {
                below(*root)
                    .into_iter()
                    .find(|node| !scene.nodes[*node].primitives.is_empty())
                    .map(|inner| (*root, inner))
            })
            .unwrap();
        let under_root = below(root);
        let mut node_tags = vec![None; scene.nodes.len()];
        node_tags[root] = Some(0);
        node_tags[inner] = Some(1);
        let tags = GlbNodeTags {
            tags: vec![
                GlbTag {
                    name: "Walls".into(),
                    visible: true,
                },
                GlbTag {
                    name: "Roof".into(),
                    visible: false,
                },
            ],
            node_tags,
        };

        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateTag {
                id: TagId(7),
                name: "Walls".into(),
                visible: true,
            }]))
            .unwrap();
        let batch =
            plan_glb_import_tagged(&document.current(), &source, "garden.glb", &[], &tags).unwrap();
        let created: Vec<_> = batch
            .commands()
            .iter()
            .filter(|command| matches!(command, CanonicalCommand::CreateTag { .. }))
            .collect();
        assert_eq!(
            created,
            [&CanonicalCommand::CreateTag {
                id: TagId(8),
                name: "Roof".into(),
                visible: false,
            }],
            "the document's own Walls tag is reused, Roof is created hidden"
        );

        let by_node = tags_by_node(&batch, &scene.traversal);
        let walls = BTreeSet::from([TagId(7)]);
        let both = BTreeSet::from([TagId(7), TagId(8)]);
        assert!(by_node.contains_key(&inner));
        for (node, tags) in &by_node {
            let expected = if *node == inner || below(inner).contains(node) {
                &both
            } else if *node == root || under_root.contains(node) {
                &walls
            } else {
                &BTreeSet::new()
            };
            assert!(tags.iter().all(|tags| tags == expected), "node {node}");
        }

        document.apply_batch(&batch).unwrap();
        let roof = document
            .current()
            .tags()
            .find(|tag| tag.name() == "Roof")
            .map(|tag| tag.id());
        assert_eq!(roof, Some(TagId(8)));
    }

    #[test]
    fn tags_that_do_not_fit_the_glb_are_refused() {
        let source = garden();
        let scene = inspect_glb(&source).unwrap();
        let snapshot = DocumentStore::new().current();
        let plan = |tags: &GlbNodeTags| {
            plan_glb_import_tagged(&snapshot, &source, "garden.glb", &[], tags).err()
        };
        let one = vec![GlbTag {
            name: "Walls".into(),
            visible: true,
        }];
        let mut beyond = vec![None; scene.nodes.len()];
        beyond[scene.traversal[0]] = Some(1);
        assert_eq!(
            plan(&GlbNodeTags {
                tags: one.clone(),
                node_tags: beyond,
            }),
            Some(GlbImportError::InvalidReference)
        );
        assert_eq!(
            plan(&GlbNodeTags {
                tags: one.clone(),
                node_tags: vec![Some(0)],
            }),
            Some(GlbImportError::InvalidReference)
        );
        let mut blank = vec![None; scene.nodes.len()];
        blank[scene.traversal[0]] = Some(0);
        assert_eq!(
            plan(&GlbNodeTags {
                tags: vec![GlbTag {
                    name: "  ".into(),
                    visible: true,
                }],
                node_tags: blank,
            }),
            Some(GlbImportError::InvalidText)
        );
        assert_eq!(plan(&GlbNodeTags::default()), None);
    }
}
