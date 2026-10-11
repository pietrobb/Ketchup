# Výsledok na Macu: Kečup Viewer

Stav k 2026-10-11. Hotové: body 1 a 2 z `MAC_TASK.md` (macOS). iOS (bod 3) ešte nezačatý.

## Prostredie

- macOS 26.6.2 (25G83), Apple Silicon, displej 1920×1080
- Xcode 26.2 (17C52)
- Rust 1.97.0 (z `rust-toolchain.toml`), ciele `aarch64-apple-darwin`, `x86_64-apple-darwin`

## 1. Overenie na macOS

```sh
cargo test -p ketchup-view-format -p ketchup-viewer -p ketchup-viewer-macos   # 42 testov, všetky prešli
cargo clippy -p ketchup-view-format -p ketchup-viewer -p ketchup-viewer-macos --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p ketchup-viewer -- --verify-open viewer/samples/kitchen.ketchup-view
# Ketchup Viewer 0.1.0 opened 20 components, 1 scenes, 0 comments
```

Prešli bez zmeny v kóde, vrátane GPU testov cez Metal. Prešli aj `scripts/check_*.py`.
`check_crate_layers.py` potrebuje Python 3.11 alebo novší (`tomllib`).

Okno sa vykresľuje cez Metal. Relácia nemala povolenie Záznam obrazovky, preto snímky
v `mac-screens/` vznikli priamo z okna cez `egui::ViewportCommand::Screenshot` (dočasný
program mimo repozitára, ktorý spustil `ViewerApp` a klikal doň). Overené kliknutím:
Izo, Celý model, výber dielu (panel s menom, rozmerom a kótami).
Neoverené ručne: otáčanie a posun myšou, dialógy `rfd` (otvorenie, uloženie pripomienok).
Pripomienky pokrývajú testy `app::tests`.

## 2. macOS aplikácia

```sh
scripts/macos/build-viewer-app.sh                      # ad-hoc podpis
scripts/macos/build-viewer-app.sh --sign "Developer ID Application: …"
```

Výstup: `artifacts/macos-viewer/<verzia>/Kečup Viewer.app` a `Ketchup-Viewer-<verzia>-macOS.dmg`.
Skript zostaví universal binárku (`lipo`, x86_64 + arm64), ikonu vyreže z `pic/splash.png`,
zapíše `Info.plist` (`sk.ketchup.viewer`, `CFBundleDocumentTypes` s rolou Viewer,
`UTExportedTypeDeclarations` `sk.ketchup.view-package` a `sk.ketchup.view-comments`), podpíše
a spustí z bundlu `--verify-open` na sample.

Otvorenie z Finderu: winit 0.30 si ponecháva delegáta aplikácie a `application:openURLs:`
nespracúva. Crate `viewer/macos` preto pri `NSApplicationWillFinishLaunching` zaregistruje
vlastnú obsluhu Apple Eventu „open documents“ (`odoc`) a súbory posiela do
`ketchup_viewer::desktop::OpenRequests`. Spoločný desktopový štart je
`crates/ketchup-viewer/src/desktop.rs`, `ketchup-viewer` (Windows) ho volá tiež.
UI ostalo bez zmeny.

Overenie cez LaunchServices (tú istú cestu ako dvojklik vo Finderi):

- `NSWorkspace URLForApplicationToOpenURL` pre `.ketchup-view` vráti `Kečup Viewer.app`.
- `open --stderr log poškodený.ketchup-view` spustí Viewer a v logu je `viewer.package.invalid`,
  teda súbor prišiel cez Apple Event pri štarte.
- Ďalší `open` poškodeného súboru, keď Viewer beží, zapíše druhú chybu do toho istého logu.
  Súbor teda dostala tá istá bežiaca inštancia a nespustila sa nová.
- `open kitchen.ketchup-view` chybu nevypíše.

Neoverené: že sa model po dvojkliku vizuálne zobrazil (nebol prístup k obrazovke),
`.ketchup-comments` cez Finder, Developer ID podpis a notarizácia (ad-hoc podpis
funguje len na tomto Macu, na inom Macu Gatekeeper appku zablokuje).

Súbor otvorený z Finderu, keď sú vo Vieweri neuložené pripomienky, najprv len ukáže
rovnaké varovanie ako tlačidlo „Otvoriť model“; až ďalšie otvorenie ich zahodí.
(Android „Otvoriť v“ ich zatiaľ nahradí bez otázky.)
