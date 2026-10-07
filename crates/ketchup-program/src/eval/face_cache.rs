//! Faces of the parts a running program asks about, kept between builtin
//! calls. A part that only gained operations since it was cached continues
//! from the cached faces, so the n-th hole drilled into a part trims its faces
//! once instead of replaying all n holes on every contact() or faces() call.
use crate::faces::FaceFrame;
use crate::model::Part;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The part as it was when its faces were computed, and those faces.
type Cached = (Part, Rc<Vec<FaceFrame>>);

#[derive(Debug, Default)]
pub(super) struct FaceCache {
    parts: RefCell<HashMap<String, Cached>>,
}

impl FaceCache {
    /// Exactly `part.face_frames()`.
    pub(super) fn face_frames(&self, part: &Part) -> Rc<Vec<FaceFrame>> {
        let mut parts = self.parts.borrow_mut();
        let faces = match parts.get(&part.name) {
            Some((cached, faces)) if cached == part => return Rc::clone(faces),
            Some((cached, faces)) if extends(cached, part) => {
                let mut faces = Vec::clone(faces);
                part.apply_face_operations(
                    &mut faces,
                    &part.operations[cached.operations.len()..],
                    true,
                );
                faces
            }
            _ => part.face_frames(),
        };
        let faces = Rc::new(faces);
        parts.insert(part.name.clone(), (part.clone(), Rc::clone(&faces)));
        faces
    }

    /// Exactly `part.face_frame(name)`.
    pub(super) fn face_frame(
        &self,
        part: &Part,
        name: &str,
    ) -> Result<FaceFrame, crate::model::FaceNameError> {
        match self.face_frames(part).iter().find(|face| face.name == name) {
            Some(face) => Ok(face.clone()),
            None => part.face_frame(name),
        }
    }
}

/// `part` is `cached` with operations appended (faces depend only on the
/// body, the placement and the operations).
fn extends(cached: &Part, part: &Part) -> bool {
    cached.body == part.body
        && cached.size_mm == part.size_mm
        && cached.at_mm == part.at_mm
        && cached.rotation == part.rotation
        && part.operations.starts_with(&cached.operations)
}
