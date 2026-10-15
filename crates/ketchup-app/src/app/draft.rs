//! Draft tool and draft analysis view. The tool drafts the selected faces of an
//! exact part about the clicked neutral face, the same `draft_faces` edit an MCP
//! client sends. The view colors exact faces by their draft along the part's +Z,
//! judged by the same analysis as `query kind=faces`; faces it cannot judge
//! exactly keep their usual color.

use super::commands::parse_angle_degrees;
use crate::*;
use ketchup_application::draft_analysis::{DraftClass, DraftQuery};

/// The pull of the draft analysis view, in part coordinates.
const VIEW_PULL: [f64; 3] = [0.0, 0.0, 1.0];

pub(crate) const fn draft_color(class: DraftClass) -> Color32 {
    match class {
        DraftClass::Positive => Color32::from_rgb(46, 160, 67),
        DraftClass::Insufficient => Color32::from_rgb(230, 190, 40),
        DraftClass::Vertical => Color32::from_rgb(235, 120, 30),
        DraftClass::Negative => Color32::from_rgb(210, 50, 50),
    }
}

impl KetchupApp {
    /// Hands a draft of the selected faces about the exact face under `pointer`
    /// to the bridge as the MCP `draft_faces` edit: one Undo step or a named refusal.
    pub(crate) fn draft_at_screen(&mut self, pointer: Pos2, rect: Rect) -> bool {
        let Some(angle_degrees) = parse_angle_degrees(&self.value_box.input) else {
            self.digest = self.catalog.text("digest-draft-invalid-angle");
            return false;
        };
        let Some(faces) = self.selected_face_references() else {
            self.digest = self.catalog.text("digest-draft-faces-required");
            return false;
        };
        let Some((neutral, _)) = self.face_at_screen(pointer, rect) else {
            self.digest = self.catalog.text("digest-draft-neutral-required");
            return false;
        };
        let operation = AssistantCadEditOperation::DraftFaces {
            definition_id: neutral.definition_id.0,
            name: self.catalog.text("model-draft-feature"),
            target_feature_id: neutral.producer_feature_id.0,
            face_reference_ids: faces.into_iter().map(|face| face.lineage_digest).collect(),
            neutral_face_reference_id: neutral.lineage_digest,
            angle_degrees,
        };
        self.queue_window_edit(operation, "digest-draft-created", None);
        true
    }

    /// The current references of the selected faces, or `None` when nothing or
    /// anything other than current faces is selected. Whether they belong to the
    /// clicked part is the planner's check.
    fn selected_face_references(&self) -> Option<Vec<TopologicalElementRef>> {
        let snapshot = self.document.current();
        let topology_results = self.topology_results_for_snapshot(&snapshot)?;
        let faces = self
            .selection
            .topological
            .iter()
            .map(|(_, selection)| {
                let resolved = selection
                    .resolve_current(&snapshot, topology_results)
                    .ok()?;
                (resolved.reference.kind == TopologicalElementKind::Face)
                    .then_some(resolved.reference)
            })
            .collect::<Option<Vec<_>>>()?;
        (!faces.is_empty()).then_some(faces)
    }

    /// The draft analysis colors of `package`'s faces by face ordinal while the
    /// view shows them; a face that is not analyzed has none.
    pub(crate) fn draft_face_colors(&self, package: &ExactBodyPackage) -> BTreeMap<u32, Color32> {
        if !self.view.contains(ViewFlag::DraftAnalysis) {
            return BTreeMap::new();
        }
        draft_classes(package)
            .into_iter()
            .map(|(ordinal, class)| (ordinal, draft_color(class)))
            .collect()
    }
}

/// The draft class of each analyzed face of `package` along the view's pull.
fn draft_classes(package: &ExactBodyPackage) -> BTreeMap<u32, DraftClass> {
    let (ExactBodyPackage::Graph(package), Some(query)) =
        (package, DraftQuery::new(VIEW_PULL, None))
    else {
        return BTreeMap::new();
    };
    package
        .face_evidence
        .iter()
        .filter_map(|evidence| Some((evidence.face_ordinal, query.class(evidence)?)))
        .collect()
}

/// The draft analysis colors the GPU scene paints, as `FaceColors`.
pub(crate) fn draft_face_rgb(package: &ExactBodyPackage) -> BTreeMap<u32, [u8; 3]> {
    draft_classes(package)
        .into_iter()
        .map(|(ordinal, class)| {
            let [red, green, blue, _] = draft_color(class).to_array();
            (ordinal, [red, green, blue])
        })
        .collect()
}
