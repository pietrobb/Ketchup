//! Tags (layers) that the reader which wrote a GLB knows for its nodes, such as a SketchUp
//! file's tags: glTF has no place for them, so they come beside the GLB. A part imports with
//! the tags of its node and of every node above it, so hiding a tag hides what the source
//! program hid with it.

use std::collections::{BTreeMap, BTreeSet};

use super::{GlbImportError, ParsedNode, display_name, validate_text};
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
/// tag, if it has one. Empty `node_tags` tags nothing. `hidden[n]` is whether node `n`
/// itself is hidden in the source file; empty `hidden` hides nothing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GlbNodeTags {
    pub tags: Vec<GlbTag>,
    pub node_tags: Vec<Option<usize>>,
    pub hidden: Vec<bool>,
}

/// The commands that create the tags no tag of the document already names, and the tags of
/// each node's parts and whether they are hidden, by glTF node index.
pub(super) struct PlannedTags {
    pub(super) commands: Vec<CanonicalCommand>,
    pub(super) by_node: Vec<BTreeSet<TagId>>,
    /// The node or one above it is hidden, so its parts import hidden, as the source shows
    /// them (review 2026-11-10 SKP-402).
    pub(super) hidden_by_node: Vec<bool>,
}

pub(super) fn plan_tags(
    snapshot: &Snapshot,
    tags: &GlbNodeTags,
    nodes: &[ParsedNode],
    traversal: &[usize],
) -> Result<PlannedTags, GlbImportError> {
    let mut parents = vec![None; nodes.len()];
    for node in traversal {
        for child in &nodes[*node].children {
            parents[*child] = Some(*node);
        }
    }
    if !tags.hidden.is_empty() && tags.hidden.len() != nodes.len() {
        return Err(GlbImportError::InvalidReference);
    }
    let mut hidden_by_node = vec![false; nodes.len()];
    for node in traversal {
        hidden_by_node[*node] = tags.hidden.get(*node).copied().unwrap_or(false)
            || parents[*node].is_some_and(|parent| hidden_by_node[parent]);
    }
    let mut planned = PlannedTags {
        commands: Vec::new(),
        by_node: vec![BTreeSet::new(); nodes.len()],
        hidden_by_node,
    };
    if tags.node_tags.is_empty() {
        return Ok(planned);
    }
    if tags.tags.len() > MAX_GLB_TAGS {
        return Err(super::excess::too_large(
            super::GlbMeasure::Tags,
            tags.tags.len(),
            MAX_GLB_TAGS,
        ));
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
    let names = shown_names(&tags.tags);
    for name in &names {
        validate_text(name)?;
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
    for node in traversal {
        // A root carries no tags from above.
        let mut carried = match parents[*node] {
            Some(parent) => planned.by_node[parent].clone(),
            None => BTreeSet::new(),
        };
        if let Some(index) = tags.node_tags[*node] {
            let name = &names[index];
            let id = match ids[index] {
                Some(id) => id,
                None => {
                    let id = if let Some(id) = existing.get(name.as_str()) {
                        *id
                    } else {
                        let id = TagId(next);
                        next = next
                            .checked_add(1)
                            .ok_or(GlbImportError::IdSpaceExhausted)?;
                        planned.commands.push(CanonicalCommand::CreateTag {
                            id,
                            name: name.clone(),
                            visible: tags.tags[index].visible,
                        });
                        id
                    };
                    // Two tags of one name are one tag, as they are in the document.
                    for (other, same) in names.iter().zip(ids.iter_mut()) {
                        if other == name {
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

/// The names the tags show in the document: as [`display_name`] shows them, "Tag 3" for a
/// third that shows nothing, and "Walls (2)" for a second that shows only as another tag
/// of a different name does (review 2026-11-13 r95 P3: a tag "\u{1}" refused the whole
/// file, and "Walls\u{7}" and "Walls\u{8}" became one tag). Tags of one name stay one.
fn shown_names(tags: &[GlbTag]) -> Vec<String> {
    let shown: Vec<String> = tags
        .iter()
        .enumerate()
        .map(|(index, tag)| {
            let name = display_name(&tag.name);
            if name.is_empty() {
                let first = tags
                    .iter()
                    .position(|other| other.name == tag.name)
                    .unwrap_or(index);
                format!("Tag {}", first + 1)
            } else {
                name
            }
        })
        .collect();
    let wanted: BTreeSet<&str> = shown.iter().map(String::as_str).collect();
    // Each name given, and the tag's own name it was given for.
    let mut given: BTreeMap<String, &str> = BTreeMap::new();
    let mut names = Vec::with_capacity(tags.len());
    for (tag, name) in tags.iter().zip(&shown) {
        let name = match given.get(name) {
            Some(own) if *own != tag.name => {
                let mut count = 2_usize;
                loop {
                    let other = format!("{name} ({count})");
                    if !given.contains_key(&other) && !wanted.contains(other.as_str()) {
                        break other;
                    }
                    count += 1;
                }
            }
            _ => name.clone(),
        };
        given.entry(name.clone()).or_insert(&tag.name);
        names.push(name);
    }
    names
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
            hidden: Vec::new(),
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
                ..GlbNodeTags::default()
            }),
            Some(GlbImportError::InvalidReference)
        );
        assert_eq!(
            plan(&GlbNodeTags {
                tags: one.clone(),
                node_tags: vec![Some(0)],
                ..GlbNodeTags::default()
            }),
            Some(GlbImportError::InvalidReference)
        );
        assert_eq!(plan(&GlbNodeTags::default()), None);
    }

    /// Review 2026-11-13 r95 P3: a tag named only with control characters or spaces
    /// refused the whole file, and two tags that differ only in a control character
    /// became one. The first imports as "Tag <n>", as an unnamed node has a name, and
    /// the second as "Walls (2)"; two tags of the very same name stay one.
    #[test]
    fn a_tag_that_shows_nothing_or_as_another_keeps_a_name_of_its_own() {
        let tag = |name: &str| GlbTag {
            name: name.into(),
            visible: true,
        };
        assert_eq!(
            shown_names(&[
                tag("\u{1}"),
                tag("  "),
                tag("Walls\u{7}"),
                tag("Walls\u{8}"),
                tag("Walls (2)"),
                tag("Walls\u{7}"),
                tag("  "),
            ]),
            [
                "Tag 1",
                "Tag 2",
                "Walls",
                "Walls (3)",
                "Walls (2)",
                "Walls",
                "Tag 2"
            ]
        );

        let source = garden();
        let scene = inspect_glb(&source).unwrap();
        let mut node_tags = vec![None; scene.nodes.len()];
        for (tag, node) in scene.traversal.iter().take(3).enumerate() {
            node_tags[*node] = Some(tag);
        }
        let tags = GlbNodeTags {
            tags: vec![tag("\u{1}"), tag("Walls\u{7}"), tag("Walls\u{8}")],
            node_tags,
            hidden: Vec::new(),
        };
        let mut document = DocumentStore::new();
        let batch =
            plan_glb_import_tagged(&document.current(), &source, "garden.glb", &[], &tags).unwrap();
        document.apply_batch(&batch).unwrap();
        let mut names: Vec<String> = document
            .current()
            .tags()
            .map(|tag| tag.name().to_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["Tag 1", "Walls", "Walls (2)"]);
    }

    /// A tag or a node named with a control character in the source file imports with the
    /// character as a space, trimmed (review 2026-11-13 P3: "Walls\u{7f}" refused the whole
    /// file as "invalid identity text").
    #[test]
    fn names_with_control_characters_import_without_them() {
        let (mut json, binary) = super::super::json_tests::sample();
        let first = usize::try_from(json["scenes"][0]["nodes"][0].as_u64().unwrap()).unwrap();
        json["nodes"][first]["name"] = "Porch\u{1b}[0m".into();
        let source = super::super::json_tests::packed(&json, &binary);
        let scene = inspect_glb(&source).unwrap();
        let mut node_tags = vec![None; scene.nodes.len()];
        node_tags[first] = Some(0);
        let tags = GlbNodeTags {
            tags: vec![GlbTag {
                name: "Walls\u{7f}".into(),
                visible: true,
            }],
            node_tags,
            hidden: Vec::new(),
        };
        let batch = plan_glb_import_tagged(
            &DocumentStore::new().current(),
            &source,
            "s.glb",
            &[],
            &tags,
        )
        .unwrap();
        let names: Vec<&str> = batch
            .commands()
            .iter()
            .filter_map(|command| match command {
                CanonicalCommand::CreateTag { name, .. }
                | CanonicalCommand::CreateGroup { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect();
        assert!(names.contains(&"Walls"), "{names:?}");
        assert!(names.contains(&"Porch [0m"), "{names:?}");
        assert!(names.iter().all(|name| !name.chars().any(char::is_control)));
        let mut document = DocumentStore::new();
        document.apply_batch(&batch).unwrap();
    }

    /// A node hidden in the source file (SketchUp's Hide on a component) imports its parts
    /// and those of every node below it hidden; the rest stay shown (review 2026-11-10
    /// SKP-402). Before, every part imported visible.
    #[test]
    fn parts_under_a_hidden_node_import_hidden() {
        let (json, binary) = super::super::json_tests::sample();
        let source = super::super::json_tests::packed(&json, &binary);
        let scene = inspect_glb(&source).unwrap();
        let root = scene.traversal[0];
        let mut under = Vec::new();
        let mut stack = vec![root];
        while let Some(node) = stack.pop() {
            under.push(node);
            stack.extend(&scene.nodes[node].children);
        }
        let mut hidden = vec![false; scene.nodes.len()];
        hidden[root] = true;
        let tags = GlbNodeTags {
            hidden,
            ..GlbNodeTags::default()
        };
        let snapshot = DocumentStore::new().current();
        let batch = plan_glb_import_tagged(&snapshot, &source, "garden.glb", &[], &tags).unwrap();
        let groups: Vec<GroupId> = batch
            .commands()
            .iter()
            .filter_map(|command| match command {
                CanonicalCommand::CreateGroup { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let (mut shown, mut hid) = (0, 0);
        for command in batch.commands() {
            if let CanonicalCommand::CreateOccurrence {
                parent: Some(parent),
                visible,
                ..
            } = command
            {
                let offset = groups.iter().position(|group| group == parent).unwrap();
                let node = scene.traversal[offset];
                assert_eq!(*visible, !under.contains(&node), "node {node}");
                if *visible { shown += 1 } else { hid += 1 }
            }
        }
        assert!(hid > 0 && shown > 0, "hidden {hid}, shown {shown}");
        assert_eq!(
            plan_glb_import_tagged(
                &snapshot,
                &source,
                "garden.glb",
                &[],
                &GlbNodeTags {
                    hidden: vec![true],
                    ..GlbNodeTags::default()
                },
            )
            .err(),
            Some(GlbImportError::InvalidReference),
            "hidden flags that do not fit the GLB's nodes are refused"
        );
    }
}
