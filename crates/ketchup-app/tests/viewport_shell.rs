use ketchup_app::{AppCommand, KetchupApp};
use ketchup_interaction::LocaleCatalog;

#[test]
fn shell_accepts_complete_real_and_pseudo_locale_catalogs() {
    let slovak = KetchupApp::with_catalog(LocaleCatalog::slovak());
    let pseudo = KetchupApp::with_catalog(LocaleCatalog::pseudo());

    assert_eq!(slovak.command_label(AppCommand::Select), "Vybrať");
    assert!(pseudo.command_label(AppCommand::Select).starts_with("[!! "));
}

#[test]
fn shell_rejects_an_incomplete_active_locale() {
    let incomplete = LocaleCatalog::parse("app-title = Kečup").unwrap();

    assert!(
        std::panic::catch_unwind(|| KetchupApp::with_catalog(incomplete)).is_err(),
        "an incomplete resource must fail before the shell can render fallback markers"
    );
}

#[test]
fn push_pull_requires_an_explicit_face_selection() {
    let mut app = KetchupApp::new();
    let initial_revision = app.document_revision();
    let initial_digest = app.canonical_digest();
    assert_eq!(app.document_height_mm(), 20.0);

    app.set_push_pull_distance_input("15 mm");
    assert!(!app.start_preview());
    assert_eq!(app.document_revision(), initial_revision);
    assert_eq!(app.canonical_digest(), initial_digest);
    assert_eq!(app.document_height_mm(), 20.0);
    assert!(app.preview_action_digest().is_none());
}

#[test]
fn cancelling_without_a_preview_preserves_the_document() {
    let mut app = KetchupApp::new();
    let initial_revision = app.document_revision();
    let initial_digest = app.canonical_digest();
    app.cancel_preview();
    assert_eq!(app.document_revision(), initial_revision);
    assert_eq!(app.canonical_digest(), initial_digest);
    assert_eq!(app.document_height_mm(), 20.0);
}

#[test]
fn desktop_shell_selects_the_wgpu_renderer() {
    let options = KetchupApp::native_options();
    assert_eq!(options.renderer, eframe::Renderer::Wgpu);
    assert_eq!(
        options.viewport.min_inner_size,
        Some(eframe::egui::Vec2::new(1_100.0, 600.0))
    );
    assert_eq!(
        options.wgpu_options.present_mode,
        eframe::wgpu::PresentMode::AutoNoVsync
    );
    let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = options.wgpu_options.wgpu_setup else {
        panic!("the desktop shell must create a bound wgpu setup");
    };
    assert_eq!(
        setup.instance_descriptor.backends,
        eframe::wgpu::Backends::DX12
    );
    assert!(setup.native_adapter_selector.is_some());
}
