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
fn preview_is_ephemeral_until_canonical_confirmation() {
    let mut app = KetchupApp::new();
    let initial_revision = app.document_revision();
    assert_eq!(app.document_height_mm(), 20.0);

    app.set_push_pull_distance_input("15 mm");
    assert!(app.start_preview());
    assert_eq!(app.document_revision(), initial_revision);
    assert_eq!(
        app.preview_action_digest().as_deref(),
        Some("Change Box-1 height from 20 mm to 35 mm")
    );

    assert!(app.confirm_preview());
    assert_eq!(app.document_revision(), initial_revision + 1);
    assert_eq!(app.document_height_mm(), 35.0);
}

#[test]
fn cancelling_preview_preserves_the_document() {
    let mut app = KetchupApp::new();
    let initial_revision = app.document_revision();
    app.set_push_pull_distance_input("15 mm");
    assert!(app.start_preview());
    app.cancel_preview();
    assert_eq!(app.document_revision(), initial_revision);
    assert_eq!(app.document_height_mm(), 20.0);
    assert!(app.preview_action_digest().is_none());
}

#[test]
fn desktop_shell_selects_the_wgpu_renderer() {
    let options = KetchupApp::native_options();
    assert_eq!(options.renderer, eframe::Renderer::Wgpu);
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
