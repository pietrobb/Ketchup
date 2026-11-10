//! The saved views a client lists with `view action=list_views`.

use super::*;

/// Saved views with the names of the tags each one hides, its display switches and
/// whether it is a Viewer scene (null while the Viewer settings are unreadable).
pub(super) fn saved_views_json(app: &KetchupApp) -> Value {
    let snapshot = app.document.current();
    let viewer = app.stored_viewer_settings().ok();
    let views = snapshot
        .saved_views()
        .map(|view| {
            let hidden = view
                .hidden_tags
                .iter()
                .filter_map(|id| snapshot.tag(*id).map(|tag| tag.name().to_owned()))
                .collect::<Vec<_>>();
            json!({"id":view.id().0,"name":view.name(),"hidden_tags":hidden,
                "style":view.style,"parallel":view.camera.parallel,
                "section":crate::app::section_json(view.section),
                "viewer":viewer.as_ref().map(|settings| settings.scene_shared(view.id()))})
        })
        .collect::<Vec<_>>();
    json!({"saved_views":views,"program_owned":program_owned(app)})
}
