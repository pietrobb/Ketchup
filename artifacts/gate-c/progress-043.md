# Gate C Desktop-Availability and Freeze-Integrity Diagnostic 043

**Status: the available Windows 11 desktop is valid for non-gating usability work but cannot replace the frozen HP-IGPU-01 notebook; the user-visible viewport repair also requires a replacement pre-observation freeze**

- Diagnostic UTC: `2026-08-02T06:39:04.0829410Z`
- Active historical freeze: `r0-v12`
- Testable assumption: `A7`
- Current `crates/ketchup-app/src/lib.rs` SHA-256: `891776d14f65d2024548f2c8fcdb0258bc3d1beef3848b379b89aa4b9c9a448a`
- R0 v12 locked application-source SHA-256: `86bddca8af98d2bab045877a8a75a3c89a381905a13e67ac86e25009a1d497c1`
- Current Gate C build-input tree SHA-256: `6f00e331e412ad06e8fbdcebd642710ff75f44ac1e24a53f897477bbbe0edabf`
- R0 v12 locked build-input tree SHA-256: `6dc2be8e1cfe992247d2946853c77977915ba249930437b6797f0b053d65b3b6`

## New availability fact

The operator reported that no Windows 11 notebook is available, but a Windows 11 desktop is available for testing. The desktop remains useful for interactive usability diagnostics, functional regression testing, and the existing `HP-DEV-01` comparison role. It is not admissible as `HP-IGPU-01`: the unchanged R0 contract requires a physical retail notebook released from 2023 through 2026, a 15-30 W mobile CPU, a battery and portable chassis, 16 GiB RAM, and an integrated Direct3D 12 GPU. Substituting or throttling a desktop would change the observed hardware class and would not support a Gate C `GO` decision.

This is evidence about testable assumption `A7`, not permission to weaken the frozen hardware profile. Gate C therefore remains externally blocked for final lower-reference measurement.

## Reproduced freeze-integrity failure

Before this diagnostic, operator feedback exposed incorrect cuboid occlusion, selected-face presentation, picking/camera disagreement, and undiscoverable Push/Pull interaction. The product source and English locale were repaired before any HP-IGPU-01 qualification or formal series. That repair changed the application source and complete build-input tree after the r0-v12 HP-DEV-01 reference observations.

The unchanged `validate-r0-v12-preregistration.ps1` now fails closed with the expected build-input-tree mismatch. The full workspace test command reaches the Gate A0 integrity wrapper and then fails for the same reason. This is the correct result: r0-v12 must not certify the repaired executable, and historical r0-v12 evidence remains untouched.

The first ordinary test attempt also could not replace `target/debug/ketchup-app.exe` because the validated interactive application was still open. A second run used an isolated target directory, preserving the user's running application; all test binaries executed before the Gate A0 integrity wrapper passed, after which the wrapper rejected r0-v12 as intended. The isolated target directory was removed after the diagnostic.

## Evidence absence

The following terminal r0-v12 paths remain absent:

- `artifacts/gate-c/report.md`
- `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v12.json`
- `artifacts/gate-c/hp-igpu-01-r0-v12-run-manifest.json`

No notebook identity, measurement result, or Gate C decision was manufactured from the desktop.

## Done-check

| Criterion | Result |
|---|---|
| `file_exists:artifacts/gate-c/report.md` | **FAIL** - correctly absent |
| `file_contains:artifacts/gate-c/report.md::GO` | **NOT EVALUABLE** - no admissible HP-IGPU-01 run exists |
| `cargo test --workspace --all-targets` | **FAIL-CLOSED** - Gate A0 rejects the repaired source under obsolete r0-v12 provenance |

## Required repair

Create a new pre-observation R0 freeze for the user-validated viewport repair while inheriting every threshold, corpus, hardware profile, oracle, and consequence byte-for-byte. Preserve all historical r0-v12 references, refresh the Gate C build-provenance chain for the new source, and generate new HP-DEV-01 references under that replacement freeze. Continue desktop usability testing independently, but keep final Gate C closure waiting for the first genuinely qualifying HP-IGPU-01 notebook.
