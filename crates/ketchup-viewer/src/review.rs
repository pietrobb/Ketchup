//! The customer's comments on the opened package: the draft being written,
//! arrows placed on parts, and the comments file returned to the author.
//! Comments never edit the package or its geometry.

use crate::{dimensions, model::Model};
use ketchup_rejection::Rejection;
use ketchup_view_format::{
    Anchor, Arrow, Comment, CommentView, Comments, InstancePath, MAX_ARROWS,
};
use std::io::{Read, Write};

/// Comments on one package and the one being written.
pub struct Review {
    pub comments: Comments,
    /// Changes not yet written to a comments file.
    pub unsaved: bool,
    pub draft: Option<Draft>,
    /// The comment whose view and arrows are shown.
    pub active: Option<u64>,
    /// The customer was told that unsaved comments would be lost.
    discard_warned: bool,
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
        Ok(Self {
            comments: Comments::new(model.package())?,
            unsaved: false,
            draft: None,
            active: None,
            discard_warned: false,
        })
    }

    /// Comments already validated against the opened package.
    pub fn with(comments: Comments) -> Self {
        Self {
            comments,
            unsaved: false,
            draft: None,
            active: None,
            discard_warned: false,
        }
    }

    /// Comments read from a returned file, checked against `model`'s package.
    pub fn open(reader: impl Read, model: &Model) -> Result<Self, Rejection> {
        Ok(Self {
            comments: Comments::read(reader, model.package())?,
            unsaved: false,
            draft: None,
            active: None,
            discard_warned: false,
        })
    }

    /// Write every comment; the package itself is never written.
    pub fn save(&mut self, writer: impl Write, model: &Model) -> Result<(), Rejection> {
        self.comments.write(writer, model.package())?;
        self.unsaved = false;
        Ok(())
    }

    /// Write a copy of the package carrying every comment.
    pub fn save_copy(&mut self, writer: impl Write, model: &Model) -> Result<(), Rejection> {
        model
            .package()
            .write_with_comments(writer, &self.comments)?;
        self.unsaved = false;
        Ok(())
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
        let id = draft.editing.unwrap_or_else(|| self.comments.next_id());
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
        self.unsaved = true;
        self.discard_warned = false;
        true
    }

    /// Remove comment `id`.
    pub fn delete(&mut self, id: u64) {
        self.comments.comments.retain(|c| c.id != id);
        if self.active == Some(id) {
            self.active = None;
        }
        self.unsaved = true;
        self.discard_warned = false;
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
        if !self.unsaved || self.discard_warned {
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
