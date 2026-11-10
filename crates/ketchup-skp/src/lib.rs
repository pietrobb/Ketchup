//! SketchUp `.skp` files through a GLB bridge.
//!
//! The OpenSKP reader (MIT, vendored in `third_party/openskp`) reads both `.skp` formats:
//! the classic archive of SketchUp 2020 and older and the container of SketchUp 2021 and
//! newer. [`skp_to_glb`] writes the model as a GLB that keeps its components and groups as
//! nodes, their placements and their materials' colours, which Ketchup's own GLB import then
//! plans with its limits. Only bytes cross this API, so no other crate depends on OpenSKP's
//! types. What the GLB cannot carry is counted in [`SkpModel::left_out`]; what the GLB import
//! then drops (faces that do not close a solid) it reports itself.
//!
//! A panic inside OpenSKP is caught and returned as [`SkpError::Panicked`] with its message.
//! The reader trusts sizes a file declares and recurses over the file's nesting, so a damaged or
//! hostile file can exhaust memory or the stack, which aborts the process: run it in a process
//! of its own.
// An error is carried, never dropped (review 2026-11-02 TST-121).
#![deny(clippy::let_underscore_must_use, clippy::map_err_ignore)]

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use openskp::{
    Definition, GlbError, InstancedGlbOptions, InstancedNode, InstancedScene, ParseOptions,
    SkpModel as ParsedModel, SkpParseError,
};

#[cfg(test)]
mod tests;

/// The SketchUp versions whose files Ketchup's tests read: 2013, 2017, 2020 (classic archive)
/// and 2025 (the container of 2021 and newer). A file of another version is read the same way
/// but reported as unverified.
pub const VERIFIED_MAJOR_VERSIONS: [u32; 4] = [13, 17, 20, 25];

/// The step of a conversion that failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    ReadSkp,
    BuildScene,
    WriteGlb,
}

impl fmt::Display for Stage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReadSkp => "reading the SketchUp file",
            Self::BuildScene => "placing the SketchUp model's components",
            Self::WriteGlb => "writing the SketchUp model as GLB",
        })
    }
}

#[derive(Debug)]
pub enum SkpError {
    /// The GLB of the model is larger than the caller's limit.
    GlbTooLarge { limit: usize, size: usize },
    /// OpenSKP refused the file: not a `.skp` file, or a damaged one.
    Read { stage: Stage, cause: SkpParseError },
    /// OpenSKP could not write the model as GLB.
    Write { cause: GlbError },
    /// OpenSKP panicked; `message` is the panic's own text.
    Panicked { stage: Stage, message: String },
}

impl fmt::Display for SkpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GlbTooLarge { limit, size } => write!(
                formatter,
                "the SketchUp model is {size} bytes as GLB, more than the {limit} bytes an \
                 import reads"
            ),
            Self::Read { stage, cause } => write!(formatter, "OpenSKP failed {stage}: {cause}"),
            Self::Write { cause } => {
                write!(formatter, "OpenSKP failed {}: {cause}", Stage::WriteGlb)
            }
            Self::Panicked { stage, message } => {
                write!(formatter, "OpenSKP panicked {stage}: {message}")
            }
        }
    }
}

impl Error for SkpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Read { cause, .. } => Some(cause),
            Self::Write { cause } => Some(cause),
            _ => None,
        }
    }
}

/// Things of one kind in the SketchUp file that the GLB does not carry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeftOut {
    /// What was left out, as an import diagnostic code: `skp_textures_dropped`, ...
    pub code: String,
    pub count: usize,
}

/// A SketchUp file read as GLB.
#[derive(Clone, Debug)]
pub struct SkpModel {
    pub glb: Vec<u8>,
    /// The SketchUp version that saved the file, for example `{25.0.575}`.
    pub version: String,
    /// Whether that version is one of [`VERIFIED_MAJOR_VERSIONS`].
    pub verified: bool,
    /// What the GLB does not carry, most frequent first; empty when it carries everything.
    pub left_out: Vec<LeftOut>,
    /// The tags some component or group carries, in the file's order. glTF has no place
    /// for them, so they come beside the GLB.
    pub tags: Vec<SkpTag>,
    /// The index in `tags` of each GLB node's tag, by glTF node index; `None` for a node on
    /// the default tag.
    pub node_tags: Vec<Option<usize>>,
}

/// A SketchUp tag (a layer before SketchUp 2020) with its visibility in the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkpTag {
    pub name: String,
    pub visible: bool,
}

/// SketchUp's default tag, shown as "Untagged": it is no tag the user made.
const DEFAULT_TAG: &str = "Layer0";

/// Reads a SketchUp file and writes its model as GLB of at most `max_glb_bytes` bytes. Textures
/// are not written: the GLB carries each material's colour.
///
/// # Errors
/// OpenSKP refused the file or the model, panicked, or the GLB is larger than the limit.
pub fn skp_to_glb(skp: &[u8], max_glb_bytes: usize) -> Result<SkpModel, SkpError> {
    let options = ParseOptions::default();
    let model = guarded(Stage::ReadSkp, || openskp::parse_skp(skp, &options))?;
    let scene = guarded(Stage::BuildScene, || {
        openskp::build_instanced_scene(skp, &options)
    })?;
    let glb = match catch_unwind(AssertUnwindSafe(|| {
        openskp::to_instanced_glb(&scene, &InstancedGlbOptions { textures: false })
    })) {
        Ok(written) => written.map_err(|cause| SkpError::Write { cause })?,
        Err(payload) => {
            return Err(SkpError::Panicked {
                stage: Stage::WriteGlb,
                message: panic_message(payload),
            });
        }
    };
    if glb.len() > max_glb_bytes {
        return Err(SkpError::GlbTooLarge {
            limit: max_glb_bytes,
            size: glb.len(),
        });
    }
    let (tags, node_tags) = node_tags(&scene);
    Ok(SkpModel {
        glb,
        verified: major_version(&model.version)
            .is_some_and(|major| VERIFIED_MAJOR_VERSIONS.contains(&major)),
        left_out: left_out(&model),
        version: model.version,
        tags,
        node_tags,
    })
}

/// The tags of the scene's nodes, by glTF node index: OpenSKP writes the nodes depth first,
/// each before its children, from the scene's root.
fn node_tags(scene: &InstancedScene) -> (Vec<SkpTag>, Vec<Option<usize>>) {
    fn visit(
        node: &InstancedNode,
        scene: &InstancedScene,
        tags: &mut Vec<SkpTag>,
        node_tags: &mut Vec<Option<usize>>,
    ) {
        let tag = (!node.layer.is_empty() && node.layer != DEFAULT_TAG).then(|| {
            tags.iter()
                .position(|tag| tag.name == node.layer)
                .unwrap_or_else(|| {
                    tags.push(SkpTag {
                        name: node.layer.clone(),
                        visible: !scene
                            .layer_hidden
                            .get(&node.layer)
                            .copied()
                            .unwrap_or(false),
                    });
                    tags.len() - 1
                })
        });
        node_tags.push(tag);
        for child in &node.children {
            visit(child, scene, tags, node_tags);
        }
    }
    let mut tags = Vec::new();
    let mut node_tags = Vec::new();
    visit(&scene.scene_hierarchy, scene, &mut tags, &mut node_tags);
    (tags, node_tags)
}

/// The major version of a SketchUp version string such as `{25.0.575}`.
fn major_version(version: &str) -> Option<u32> {
    version
        .trim_start_matches('{')
        .split('.')
        .next()?
        .parse()
        .ok()
}

/// What of `model` the GLB does not carry, counted by kind, most frequent first.
fn left_out(model: &ParsedModel) -> Vec<LeftOut> {
    let definitions = || std::iter::once(model.root()).chain(model.definitions.values());
    let sum = |count: fn(&Definition) -> usize| definitions().map(count).sum::<usize>();
    let mut left_out: Vec<LeftOut> = [
        (
            "skp_textures_dropped",
            model
                .materials
                .iter()
                .filter(|material| material.texture.is_some())
                .count(),
        ),
        // A part takes the tags of its component or group; a face's own tag has no part.
        (
            "skp_face_tags_dropped",
            sum(|d| {
                d.faces
                    .values()
                    // A classic-format face names no tag, only its id: 0 for the default.
                    .filter(|face| match face.layer.as_str() {
                        "" => face.layer_id.is_some_and(|id| id != 0),
                        name => name != DEFAULT_TAG,
                    })
                    .count()
            }),
        ),
        ("skp_scenes_dropped", model.pages.len()),
        (
            "skp_dimensions_and_text_dropped",
            model.dimensions.len() + sum(|d| d.dimensions.len() + d.texts.len()),
        ),
        (
            "skp_section_planes_dropped",
            sum(|d| d.section_planes.len()),
        ),
        (
            "skp_guides_dropped",
            sum(|d| d.construction_lines.len() + d.construction_points.len()),
        ),
        (
            "skp_loose_edges_dropped",
            sum(|d| d.loose_edge_runs().len()),
        ),
        (
            "skp_images_dropped",
            model.definitions.values().filter(|d| d.is_image).count(),
        ),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(code, count)| LeftOut {
        code: code.to_owned(),
        count,
    })
    .collect();
    left_out.sort_by(|a, b| b.count.cmp(&a.count).then(a.code.cmp(&b.code)));
    left_out
}

/// Runs one OpenSKP step, keeping its error as the cause and turning a panic into
/// [`SkpError::Panicked`] with the panic's message.
fn guarded<T>(stage: Stage, run: impl FnOnce() -> Result<T, SkpParseError>) -> Result<T, SkpError> {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(result) => result.map_err(|cause| SkpError::Read { stage, cause }),
        Err(payload) => Err(SkpError::Panicked {
            stage,
            message: panic_message(payload),
        }),
    }
}

/// The text a panic was raised with (`panic!` gives a `&str` or a `String`).
fn panic_message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => payload.downcast_ref::<&str>().map_or_else(
            || String::from("the panic carried no text"),
            |message| (*message).to_owned(),
        ),
    }
}
