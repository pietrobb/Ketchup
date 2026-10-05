use crate::*;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;

const SOURCE: &str =
    "box(\"stena\", (3000, 160, 2500), material = \"drevo\", tags = [\"konštrukcia\"])\n";

fn harness() -> egui_kittest::Harness<'static, KetchupApp> {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "house.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(3);
    harness
}

/// The glyph literal of every `icon_button(...)` call in the app sources.
fn icon_glyphs() -> Vec<(String, String)> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app");
    let mut glyphs = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.to_string_lossy().ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for call in text.split("icon_button(").skip(1) {
            let Some(arguments) = call.split(';').next() else {
                continue;
            };
            if let Some(glyph) = arguments.split('"').nth(1) {
                let file = path.file_name().unwrap().to_string_lossy().into_owned();
                glyphs.push((file, glyph.to_owned()));
            }
        }
    }
    glyphs
}

/// Every icon is drawn by the proportional UI font; a glyph it lacks paints
/// as an empty box that tells the user nothing.
#[test]
fn every_icon_button_glyph_exists_in_the_ui_font() {
    let harness = harness();
    let glyphs = icon_glyphs();
    assert!(glyphs.len() >= 20, "{glyphs:?}");
    let font = egui::FontId::proportional(14.0);
    let missing: Vec<_> = glyphs
        .iter()
        .filter(|(_, glyph)| !harness.ctx.fonts(|fonts| fonts.has_glyphs(&font, glyph)))
        .collect();
    assert!(missing.is_empty(), "glyphs without a font: {missing:?}");
    assert!(
        !harness.ctx.fonts(|fonts| fonts.has_glyphs(&font, "◇")),
        "the check must be able to see a missing glyph"
    );
}

/// Hovering an icon-only layer button shows what it does, also when the
/// button is disabled.
#[test]
fn layer_icon_buttons_explain_themselves_on_hover() {
    let mut harness = harness();
    for key in ["tags-hide-all", "tags-select-untagged"] {
        let name = harness.state().catalog.text(key);
        harness.get_by_role_and_label(Role::Button, &name).hover();
        harness.run_steps(60);
        let shown = harness.query_all_by_label(&name).count();
        assert!(shown >= 2, "{key}: no hover hint (found {shown} nodes)");
    }
}
