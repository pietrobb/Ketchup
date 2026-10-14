//! The `.ketchup-comments` return package: a customer's comments on one
//! exported `.ketchup-view`, each with the view it was written in, the picked
//! component and 3D arrows. Bounded JSON that never carries or edits geometry.

use crate::{Anchor, Camera, DisplayStyle, InstancePath, Package, Section, Source, reject};
use ketchup_rejection::Rejection;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::hash::BuildHasher;
use std::io::{Read, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Largest accepted comments file.
pub const MAX_COMMENTS_BYTES: usize = 4 * 1024 * 1024;
/// Most comments in one file.
pub const MAX_COMMENTS: usize = 1_000;
/// Most arrows on one comment.
pub const MAX_ARROWS: usize = 32;
/// Longest comment text.
pub const MAX_COMMENT_TEXT_BYTES: usize = 16 * 1024;
/// Longest author name.
pub const MAX_AUTHOR_BYTES: usize = 256;
const FORMAT: &str = "ketchup-comments";
const VERSION: u32 = 1;
/// Hex digits of a SHA-256 checksum.
const SHA256_HEX_DIGITS: usize = 64;

/// All comments on one exported package.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comments {
    pub format: String,
    pub version: u32,
    pub export: ExportRef,
    pub comments: Vec<Comment>,
}

/// The package the comments belong to: its source identity and the
/// checksum its envelope already carries, not a new fingerprint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportRef {
    pub source: Source,
    pub package_sha256: String,
}

/// One comment and the view it was written in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comment {
    pub id: u64,
    pub author: Option<String>,
    pub text: String,
    pub view: CommentView,
    pub selected: Option<InstancePath>,
    pub arrows: Vec<Arrow>,
}

/// Camera and what was shown, so another screen restores the same intent.
/// What was shown is `shown` (only these, after an isolation) or else the
/// `scene`'s visibility (all when none), less `hidden`; see
/// [`CommentView::set_visibility`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommentView {
    pub camera: Camera,
    pub hidden: Vec<InstancePath>,
    pub style: DisplayStyle,
    pub section: Option<Section>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown: Option<Vec<InstancePath>>,
}

/// An arrow from `tail` to `tip`, each held on a component or in the world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arrow {
    pub tip: Anchor,
    pub tail: Anchor,
}

impl Package {
    /// SHA-256 of the manifest and geometry exactly as [`Package::write`]
    /// checksums them: the identity of this export.
    pub fn payload_sha256(&self) -> Result<String, Rejection> {
        use sha2::{Digest, Sha256};
        struct Hashing(Sha256);
        impl Write for Hashing {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.update(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut hashing = Hashing(Sha256::new());
        serde_json::to_writer(&mut hashing, &self.manifest).map_err(|error| {
            reject("manifest", "The manifest could not be encoded.").caused_by(error)
        })?;
        hashing.0.update(&self.glb);
        Ok(hashing
            .0
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

impl Comments {
    /// No comments yet on `package`.
    pub fn new(package: &Package) -> Result<Self, Rejection> {
        Ok(Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            export: ExportRef {
                source: package.manifest.source.clone(),
                package_sha256: package.payload_sha256()?,
            },
            comments: vec![],
        })
    }

    /// Read comments for `package`; size is limited before parsing and
    /// comments on another export or version are refused.
    pub fn read(reader: impl Read, package: &Package) -> Result<Self, Rejection> {
        let comments = Self::read_unbound(reader)?;
        comments.validate(package)?;
        Ok(comments)
    }

    /// Read comments without their package, as the author does: everything but
    /// the binding to the export is checked; the caller matches the source.
    pub fn read_unbound(reader: impl Read) -> Result<Self, Rejection> {
        let mut bytes = Vec::new();
        reader
            .take(u64::try_from(MAX_COMMENTS_BYTES + 1).expect("bounded size fits u64"))
            .read_to_end(&mut bytes)
            .map_err(|error| {
                reject("comments", "The comments file could not be read.").caused_by(error)
            })?;
        if bytes.len() > MAX_COMMENTS_BYTES {
            return Err(reject("comments", "The comments file exceeds 4 MiB."));
        }
        let comments: Self = serde_json::from_slice(&bytes).map_err(|error| {
            reject("comments", "The comments file is malformed.").caused_by(error)
        })?;
        comments.validate_contents()?;
        Ok(comments)
    }

    /// Validate against `package`, then write; nothing is written on failure.
    pub fn write(&self, mut writer: impl Write, package: &Package) -> Result<(), Rejection> {
        self.validate(package)?;
        let bytes = serde_json::to_vec(self).map_err(|error| {
            reject("comments", "The comments could not be encoded.").caused_by(error)
        })?;
        if bytes.len() > MAX_COMMENTS_BYTES {
            return Err(reject("comments", "The comments exceed 4 MiB."));
        }
        writer.write_all(&bytes).map_err(|error| {
            reject("output", "The comments file could not be written.").caused_by(error)
        })
    }

    /// A new comment ID no comment here has. IDs are random, not counted from
    /// 1, so the comments two customers write on one export keep apart when
    /// the author imports both: the author matches comments by export and ID.
    pub fn new_id(&self) -> u64 {
        loop {
            let id = random_id();
            if id != 0 && !self.comments.iter().any(|c| c.id == id) {
                return id;
            }
        }
    }

    /// Every check except the binding to one export: format, limits, text,
    /// finite cameras, sections and arrow points.
    fn validate_contents(&self) -> Result<(), Rejection> {
        if self.format != FORMAT || self.version != VERSION {
            return Err(reject(
                "comments.version",
                "Unknown comments format or version.",
            ));
        }
        if self.export.package_sha256.len() != SHA256_HEX_DIGITS
            || !self
                .export
                .package_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
        {
            return Err(reject(
                "comments.export",
                "The export checksum is malformed.",
            ));
        }
        if self.comments.len() > MAX_COMMENTS {
            return Err(reject("comments", "Too many comments in one file."));
        }
        let mut ids = BTreeSet::new();
        for comment in &self.comments {
            if comment.id == 0 || !ids.insert(comment.id) {
                return Err(reject(
                    "comments.id",
                    "Comment IDs must be unique and nonzero.",
                ));
            }
            check_text(&comment.text, MAX_COMMENT_TEXT_BYTES, "comments.text")?;
            if comment.text.trim().is_empty() {
                return Err(reject("comments.text", "A comment has no text."));
            }
            if let Some(author) = &comment.author {
                check_text(author, MAX_AUTHOR_BYTES, "comments.author")?;
            }
            comment.view.camera.validate()?;
            if let Some(section) = &comment.view.section {
                let length = section.normal.iter().map(|v| v * v).sum::<f64>().sqrt();
                if !(length.is_finite() && length > 0.0 && section.offset_mm.is_finite()) {
                    return Err(reject("comments.view.section", "The section is invalid."));
                }
            }
            if comment.view.hidden.len() > crate::container::MAX_ENTRIES
                || comment.view.shown.as_ref().map_or(0, Vec::len) > crate::container::MAX_ENTRIES
            {
                return Err(reject(
                    "comments.view.hidden",
                    "Too many hidden components.",
                ));
            }
            if comment.arrows.len() > MAX_ARROWS {
                return Err(reject("comments.arrows", "Too many arrows on one comment."));
            }
            for anchor in comment.arrows.iter().flat_map(|a| [&a.tip, &a.tail]) {
                crate::validation::point(&anchor.point_mm)?;
            }
        }
        Ok(())
    }

    /// The contents, then the binding to `package` and its components.
    fn validate(&self, package: &Package) -> Result<(), Rejection> {
        self.validate_contents()?;
        if self.export.source != package.manifest.source
            || self.export.package_sha256 != package.payload_sha256()?
        {
            return Err(reject(
                "comments.export",
                "These comments belong to a different export of the model.",
            ));
        }
        let paths: BTreeSet<_> = package
            .manifest
            .occurrences
            .iter()
            .map(|o| &o.path)
            .collect();
        for comment in &self.comments {
            let view = &comment.view;
            for hidden in view.hidden.iter().chain(view.shown.iter().flatten()) {
                // A hidden path may be an assembly above the bodies.
                if !paths
                    .iter()
                    .any(|path| crate::visibility::covered(std::slice::from_ref(hidden), path))
                {
                    return Err(unknown_path("comments.view.hidden"));
                }
            }
            if let Some(id) = view.scene
                && !package.manifest.scenes.iter().any(|scene| scene.id == id)
            {
                return Err(reject(
                    "comments.view.scene",
                    "A comment refers to a scene that is not in this package.",
                ));
            }
            let picked = comment.selected.iter();
            let anchored = comment
                .arrows
                .iter()
                .flat_map(|a| [&a.tip, &a.tail])
                .filter_map(|anchor| anchor.occurrence.as_ref());
            if picked.chain(anchored).any(|path| !paths.contains(path)) {
                return Err(unknown_path("comments.arrows"));
            }
        }
        Ok(())
    }
}

fn check_text(text: &str, limit: usize, target: &str) -> Result<(), Rejection> {
    if text.len() > limit || text.contains('\0') {
        return Err(reject(
            target,
            "The text is too long or contains a NUL character.",
        ));
    }
    Ok(())
}

/// The state new comment IDs are drawn from in this process.
static ID_STREAM: OnceLock<AtomicU64> = OnceLock::new();

/// Seed the comment IDs of this process. A host whose Rust standard library
/// has no randomness of its own (the browser) calls it once, before any
/// comment is written; elsewhere the operating system seeds it.
pub fn seed_comment_ids(seed: u64) {
    let _ = ID_STREAM.set(AtomicU64::new(seed));
}

/// A random ID below 2^53, exact in every JSON reader (SplitMix64).
fn random_id() -> u64 {
    const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;
    let stream = ID_STREAM.get_or_init(|| {
        AtomicU64::new(std::collections::hash_map::RandomState::new().hash_one(GOLDEN))
    });
    let mut z = stream
        .fetch_add(GOLDEN, Ordering::Relaxed)
        .wrapping_add(GOLDEN);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) >> 11
}

fn unknown_path(target: &str) -> Rejection {
    reject(
        target,
        "A comment refers to a component that is not in this package.",
    )
}
