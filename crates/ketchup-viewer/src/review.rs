//! The customer's comments on the opened package: the draft being written,
//! arrows placed on parts, and the comments file returned to the author.
//! Comments never edit the package or its geometry.

use crate::{dimensions, model::Model};
use ketchup_rejection::Rejection;
pub use ketchup_view_format::seed_comment_ids;
use ketchup_view_format::{
    Anchor, Arrow, Comment, CommentView, Comments, InstancePath, MAX_ARROWS,
};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// Comments on one package and the one being written.
pub struct Review {
    pub comments: Comments,
    /// Changes with every edit of `comments`; unique in the process, so a
    /// file written from another Review never counts as saving this one.
    revision: u64,
    /// The revision last confirmed written to a file.
    saved_revision: u64,
    pub draft: Option<Draft>,
    /// The comment whose view and arrows are shown.
    pub active: Option<u64>,
    /// The customer was told that unsaved comments would be lost.
    discard_warned: bool,
}

/// Comments written to bytes the host has still to keep in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use = "pass it to `written` once the file is kept"]
pub struct Written(u64);

fn next_revision() -> u64 {
    static REVISIONS: AtomicU64 = AtomicU64::new(0);
    REVISIONS.fetch_add(1, Ordering::Relaxed) + 1
}

/// A comment being written or edited.
pub struct Draft {
    /// The comment it replaces when saved; `None` for a new one.
    pub editing: Option<u64>,
    pub author: String,
    pub text: String,
    pub view: CommentView,
    pub selected: Option<InstancePath>,
    pub arrows: Vec<Arrow>,
    /// While placing an arrow: its tip once picked.
    pub placing: Option<Option<Anchor>>,
}

impl Review {
    /// No comments yet on the package `model` shows.
    pub fn new(model: &Model) -> Result<Self, Rejection> {
        Ok(Self::with(Comments::new(model.package())?))
    }

    /// Comments already validated against the opened package.
    pub fn with(comments: Comments) -> Self {
        let revision = next_revision();
        Self {
            comments,
            revision,
            saved_revision: revision,
            draft: None,
            active: None,
            discard_warned: false,
        }
    }

    /// Comments read from a returned file, checked against `model`'s package.
    pub fn open(reader: impl Read, model: &Model) -> Result<Self, Rejection> {
        Comments::read(reader, model.package()).map(Self::with)
    }

    /// Write every comment; the package itself is never written. The
    /// comments count as saved only once the host confirms the file with
    /// [`Self::written`]: a save dialog the customer cancels saves nothing.
    pub fn save(&self, writer: impl Write, model: &Model) -> Result<Written, Rejection> {
        self.comments.write(writer, model.package())?;
        Ok(Written(self.revision))
    }

    /// Write a copy of the package carrying every comment; saved as [`Self::save`].
    pub fn save_copy(&self, writer: impl Write, model: &Model) -> Result<Written, Rejection> {
        model
            .package()
            .write_with_comments(writer, &self.comments)?;
        Ok(Written(self.revision))
    }

    /// The host kept the file `written` came from: the comments as they were
    /// then are saved; changes made since stay unsaved.
    pub fn written(&mut self, written: Written) {
        if written.0 == self.revision {
            self.saved_revision = self.revision;
        }
    }

    /// Changes not yet confirmed written to a file.
    pub fn unsaved(&self) -> bool {
        self.revision != self.saved_revision
    }

    /// Start a new comment on what is shown now; the last author is offered.
    pub fn start(&mut self, view: CommentView, selected: Option<InstancePath>) {
        self.draft = Some(Draft {
            editing: None,
            author: self
                .comments
                .comments
                .last()
                .and_then(|c| c.author.clone())
                .unwrap_or_default(),
            text: String::new(),
            view,
            selected,
            arrows: vec![],
            placing: None,
        });
    }

    /// Edit comment `id`, keeping the view it was written in.
    pub fn edit(&mut self, id: u64) {
        if let Some(comment) = self.comments.comments.iter().find(|c| c.id == id) {
            self.draft = Some(Draft {
                editing: Some(comment.id),
                author: comment.author.clone().unwrap_or_default(),
                text: comment.text.clone(),
                view: comment.view.clone(),
                selected: comment.selected.clone(),
                arrows: comment.arrows.clone(),
                placing: None,
            });
        }
    }

    /// Keep the draft as a comment; `false` while it has no text.
    pub fn commit(&mut self) -> bool {
        let Some(draft) = self.draft.take_if(|draft| !draft.text.trim().is_empty()) else {
            return false;
        };
        let id = draft.editing.unwrap_or_else(|| self.comments.new_id());
        let author = draft.author.trim();
        let comment = Comment {
            id,
            author: (!author.is_empty()).then(|| author.to_owned()),
            text: draft.text.trim().to_owned(),
            view: draft.view,
            selected: draft.selected,
            arrows: draft.arrows,
        };
        match self.comments.comments.iter_mut().find(|c| c.id == id) {
            Some(existing) => *existing = comment,
            None => self.comments.comments.push(comment),
        }
        self.active = Some(id);
        self.changed();
        true
    }

    fn changed(&mut self) {
        self.revision = next_revision();
        self.discard_warned = false;
    }

    /// Remove comment `id`.
    pub fn delete(&mut self, id: u64) {
        self.comments.comments.retain(|c| c.id != id);
        if self.active == Some(id) {
            self.active = None;
        }
        self.changed();
    }

    /// Place the next arrow end at a picked point: the tip first, then the
    /// tail. `hit` is the picked occurrence and point; `free` the point on
    /// the view plane used when the tail is not on a part.
    pub fn place_arrow_end(
        &mut self,
        model: &Model,
        hit: Option<(usize, [f64; 3])>,
        free: [f64; 3],
    ) {
        let Some(draft) = &mut self.draft else {
            return;
        };
        let Some(placing) = &mut draft.placing else {
            return;
        };
        match placing.take() {
            None => {
                // The tip points at a part.
                *placing =
                    hit.and_then(|(index, point)| dimensions::anchor_on(model, index, point));
            }
            Some(tip) => {
                let tail = match hit {
                    Some((index, point)) => dimensions::anchor_on(model, index, point),
                    None => match tip.occurrence.as_ref().and_then(|path| {
                        model
                            .manifest()
                            .occurrences
                            .iter()
                            .position(|o| &o.path == path)
                    }) {
                        Some(index) => dimensions::anchor_on(model, index, free),
                        None => Some(Anchor {
                            occurrence: None,
                            point_mm: free,
                        }),
                    },
                };
                if let Some(tail) = tail
                    && draft.arrows.len() < MAX_ARROWS
                {
                    draft.arrows.push(Arrow { tip, tail });
                }
                draft.placing = None;
            }
        }
    }

    /// Whether unsaved comments may be dropped now: the first attempt only
    /// warns, a repeated one proceeds.
    pub fn may_discard(&mut self) -> bool {
        if !self.unsaved() || self.discard_warned {
            return true;
        }
        self.discard_warned = true;
        false
    }

    /// Arrows to draw: the draft's while writing, else the shown comment's.
    pub fn visible_arrows(&self) -> &[Arrow] {
        if let Some(draft) = &self.draft {
            return &draft.arrows;
        }
        self.active
            .and_then(|id| self.comments.comments.iter().find(|c| c.id == id))
            .map_or(&[], |c| c.arrows.as_slice())
    }
}
