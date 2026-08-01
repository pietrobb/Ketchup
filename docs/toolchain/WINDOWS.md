# Reproducible Windows toolchain

This baseline builds Ketchup with Rust 1.97.0 and a source-built, replaceable OCCT 8.0.1 shared-library backend. Source, build, and install trees are ignored; the scripts and normalized fingerprint artifacts are versionable evidence.

## Frozen reference environment

- Windows 10 x86-64, build 19045.
- Visual Studio Build Tools 2022 17.5.4 (`17.5.33530.505`).
- MSVC compiler 19.35, toolset `14.35.32215`.
- Windows SDK `10.0.22000.0`; its x64 `rc.exe` and `mt.exe` are selected explicitly.
- CMake 4.2.1 binary SHA-256 `56a4d1e9407238ab004abc6a0bb960aa10a8a77b0c52023e10cdf880fe16346f`.
- Rust 1.97.0 and cargo-deny 0.20.2.
- Git 2.41.0.windows.1 binary SHA-256 `5ecc74f73bcb2ed9ca3c35e7fa287018147fa53c5f8f402517af675a14afbb1a`, plus GitHub CLI and Rustup.

OCCT supports Visual Studio 2019 or later, so upgrading the installed compiler was unnecessary. This R0 fingerprint is intentionally stricter than OCCT's compatibility range: the build script rejects different compiler, linker, SDK, resource-tool, manifest-tool, or CMake binaries.

## Reproduce

Start with absent or empty `third_party/occt-build-r0-v1` and `third_party/occt-install-r0-v1` directories. Run from the repository root in PowerShell:

```powershell
rustup toolchain install 1.97.0-x86_64-pc-windows-msvc --profile minimal --component clippy rustfmt
cargo install cargo-deny --version 0.20.2 --locked
gh repo clone Open-Cascade-SAS/OCCT third_party/occt-src -- --branch V8.0.1 --depth 1
./scripts/windows/build-occt.ps1
./scripts/windows/capture-toolchain.ps1
./scripts/windows/validate-toolchain.ps1
```

The build script refuses non-empty output directories, a modified or wrong-origin OCCT checkout, any source commit other than `b8f597c677811d1f9f4d8a97f5ae2825c0353a42`, and any native tool outside the frozen fingerprint. For another clean run, provide different empty `-BuildDir` and `-InstallDir` paths below `third_party` to all three scripts. Upstream `V8.0.1` is a lightweight, unsigned Git tag; the pinned commit and expected vendor origin provide content identity but not cryptographic publisher attestation.

The build enables Foundation Classes, Modeling Data, Modeling Algorithms, and Data Exchange in Release configuration as shared libraries. OCCT still resolves toolkits required transitively by those modules, so the manifest records the actual complete DLL closure. Visualization, Application Framework, DRAW, external integrations, and release-mode exception suppression are disabled at module/configuration level, and no `TKOpenGl.dll` renderer is produced.

## Evidence and validation

- `artifacts/r0/occt-build-manifest.json` records source provenance, exact selected-tool hashes, the raw local CMake-cache hash, every installed DLL SHA-256, and a deterministic exact-set fingerprint over every installed header, import library, resource, notice, and binary.
- `artifacts/r0/occt-cmake-config.json` records an allowlisted, path-independent semantic configuration. The raw cache is not published because it contains machine-specific absolute paths.
- `tests/native/occt-smoke.cpp` is compiled, linked against the installed OCCT import libraries, and run with the installed DLLs; it creates and validates an exact box.
- `Cargo.lock` freezes the Rust dependency graph.
- `deny.toml` blocks unknown sources and licenses outside the R0 allowlist. The RustSec advisory database is checked at validation time and is not yet revision-pinned; this is a current vulnerability scan, not immutable historical evidence.
- Validation requires exact equality between the manifest DLL set and the clean install tree; missing, changed, duplicate, traversing, or extra DLL paths fail.

The artifacts prove one clean build, the exact resulting OCCT SDK tree, and the selected executable fingerprints. They do not fingerprint the complete MSBuild/MSVC/Windows SDK support-file closure and do not claim byte-for-byte reproducibility across servicing states, checkout paths, timestamps, or filesystems. A second independent clean build with a fully captured build-input closure is required before claiming bitwise reproducibility.
