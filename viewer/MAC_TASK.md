# Úloha pre inštanciu na Macu: Kečup Viewer pre macOS a iPhone

Kečup Viewer je samostatná Rust aplikácia na offline prezeranie súborov
`.ketchup-view` (export z Kečupu) a `.ketchup-comments` (pripomienky zákazníka).
Windows verzia, webová verzia a Android aplikácia sú hotové (Android overený
v emulátore). Chýba macOS a iOS.
Tvoja úloha: postaviť a reálne spustiť obe verzie na tomto Macu, opraviť, čo
treba, a výsledok pushnúť späť. Pravidlá repozitára sú v `AGENTS.md`.

## Čo už existuje

- `crates/ketchup-view-format` — čítanie a validácia balíka (bez GPU, bez UI).
- `crates/ketchup-viewer` — celé UI a 3D zobrazenie (eframe/egui 0.32.3,
  wgpu 25.0.2). Na macOS/iOS je v `Cargo.toml` už zapnutý wgpu backend
  `metal`, na macOS aj `rfd` (dialóg na otvorenie súboru) a `accesskit`.
  Binárka `ketchup-viewer` (`src/main.rs`) berie cestu k súboru ako argument
  a má `--verify-open <súbor>` (otvorí bez okna a vypíše počty).
- `viewer/android` — Android aplikácia, vzor pre iOS vrstvu: `src/lib.rs`
  (štart, odsadenie od systémových líšt a klávesnice), `src/files.rs` +
  `java/.../ViewerActivity.java` (systémový výber súboru, „Uložiť ako“,
  „Otvoriť v“/zdieľanie z iných aplikácií; balík od komentárov rozlišuje
  `Package::has_signature`). Na iOS tomu zodpovedá `UIDocumentPickerViewController`
  a `CFBundleDocumentTypes`. Build: `scripts/windows/build-android-viewer.ps1`.
- `viewer/web` — webová verzia (pre Mac netreba, ale Safari test je vítaný).
- `viewer/samples/kitchen.ketchup-view` — skutočný export (20 dielov, 1 scéna).

Viewer nesmie závisieť od `ketchup-app`, OCCT, programu, AI ani MCP. Na Macu
teda netreba stavať celý Kečup — stačia tieto crate.

## 1. Overenie na macOS (najprv toto)

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo test -p ketchup-view-format -p ketchup-viewer
cargo clippy -p ketchup-view-format -p ketchup-viewer --all-targets -- -D warnings
cargo run -p ketchup-viewer -- --verify-open viewer/samples/kitchen.ketchup-view
cargo run -p ketchup-viewer -- viewer/samples/kitchen.ketchup-view
```

GPU testy v `crates/ketchup-viewer/src/gpu_tests.rs` kreslia skutočné pixely —
na Macu idú cez Metal. Ak niečo padne iba na Metale, oprav to v
`crates/ketchup-viewer` tak, aby Windows/web ostali nezmenené.

V okne ručne skús: otáčanie, posun, zoom, „prispôsobiť“, klik na diel (výber
a panel s údajmi), prepnutie scény, kóty, napísanie komentára a jeho uloženie
ako `.ketchup-comments` a opätovné otvorenie toho súboru.

## 2. macOS aplikácia `.app`

- Release build ako universal binárka (arm64 + x86_64, `lipo`).
- Bundle `Kečup Viewer.app` s `Info.plist`: `CFBundleIdentifier`
  (napr. `sk.ketchup.viewer`), ikona, `CFBundleDocumentTypes` a
  `UTExportedTypeDeclarations` pre `.ketchup-view` a `.ketchup-comments`
  (rola Viewer).
- **Pozor:** na macOS Finder neposiela súbor ako argument príkazového riadka,
  ale Apple Eventom (`application:openURLs:`). Treba to doplniť, aby
  dvojklik na `.ketchup-view` a „Otvoriť v…“ naozaj otvorili súbor, aj keď
  Viewer už beží. Riešenie drž v tenkej platformovej vrstve, nie v UI.
- Skript na zostavenie bundle a `.dmg` daj do `scripts/macos/` (vzor:
  `scripts/windows/build-manual-alpha.ps1`, ktorý robí to isté pre Windows).
- Podpis: lokálne stačí ad-hoc (`codesign -s -`). Developer ID a notarizáciu
  rob iba ak ich na Macu máš nastavené; inak ich nechaj ako ďalší krok.
- Overenie: zostav `.app`, spusti
  `"Kečup Viewer.app/Contents/MacOS/ketchup-viewer" --verify-open viewer/samples/kitchen.ketchup-view`
  a otvor sample dvojklikom vo Finderi.

## 3. iPhone (iOS)

Cieľ: Viewer beží na iPhone (simulátor aj reálne zariadenie, ak je dostupné),
zobrazí model, otáčanie/zoom gestami, výber dielu a otvorenie lokálneho súboru.

- `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
- Spoločné UI je `ketchup_viewer::app::ViewerApp` (eframe `App`). eframe nemá
  oficiálnu iOS podporu; over najprv, či ide (winit má iOS). Ak nie, sprav
  tenkú iOS vrstvu: winit + `egui-wgpu` + `egui-winit` priamo, a volaj ten istý
  `ViewerApp`. UI sa nesmie rozdeliť na dve verzie.
- Platformový projekt daj do `viewer/ios/` (Rust `staticlib` crate podobný
  `viewer/android` + Xcode projekt, prípadne vygenerovaný cez xcodegen).
- Otvorenie súboru: `UIDocumentPickerViewController` a „Otvoriť v Kečup
  Viewer“ z Files/Mail/AirDrop (`CFBundleDocumentTypes` + `LSSupportsOpeningDocumentsInPlace`).
  Uloženie pripomienok: zdieľanie `.ketchup-comments` cez share sheet —
  `ViewerApp::with_host_sharing()` ukáže tlačidlo „Poslať pripomienky autorovi“
  a pošle `FileRequest::ShareComments` (vzor: `viewer/android/src/files.rs`).
- Rozloženie pre úzku obrazovku už existuje (model hore, panel dole); over na
  výške aj šírke a s bezpečnými okrajmi (notch/home indicator).
- Podpis pre reálny iPhone: tvoj vývojársky tím v Xcode. App Store ani
  TestFlight nepublikuj.

## Hotovo, keď

1. `cargo test` a `clippy` pre `ketchup-view-format` a `ketchup-viewer` prejdú na Macu.
2. `.app` sa zostaví skriptom, otvorí sample dvojklikom aj cez `--verify-open`.
3. iOS build beží aspoň v simulátore so sample súborom: model, gestá, výber.
4. Výsledok zapíš do `viewer/MAC_RESULT.md`: verzie (macOS, Xcode, Rust),
   presné príkazy, čo prešlo, čo **nebolo** overené (napr. reálny iPhone,
   notarizácia) a prípadné screenshoty do `viewer/mac-screens/`.
5. Commit + push na `main`. Žiadne zmeny mimo Viewera bez dôvodu.

Ak niečo zablokuje (chýba Xcode, účet, zariadenie), zapíš to presne do
`MAC_RESULT.md` a dokonči, čo sa dá.
