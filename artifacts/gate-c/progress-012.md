# Gate C Runner Closure Repair 012

**Status: the HP-IGPU-01 runner now fails closed over the complete observed benchmark binaries and frozen OCCT runtime**

- Observation UTC: `2026-08-02`
- Active freeze: `r0-v11`
- Testable assumption: `A7`
- Repaired runner SHA-256: `b1e3fc92821a11572b30c4fc8366b1ed0bd26583a401cfcfa94b3124a88f6c9e`

## Diagnostic finding

The qualification runner previously checked the R0 v11 lock and four direct benchmark source files, but the release executables also depend on local workspace libraries, manifests, the C++ exact bridge, the embedded locale, and dynamically loaded OCCT libraries. A change in that larger dependency closure could therefore reach an HP-IGPU-01 build without being rejected by the runner's four-file check.

A fresh release rebuild before this repair reproduced all three immutable HP-DEV-01 executable hashes exactly:

| Executable | Rebuilt SHA-256 | HP-DEV-01 provenance |
|---|---|---|
| `ketchup-gate-c-core.exe` | `cfea07b32dec2ee395bb26aa5a7c3dce7d8411190361d821696063a049f680c5` | exact match |
| `ketchup-exact-worker.exe` | `54cf12f88fd2fdb1d69de23cccc36c80a6111507c4e75217f5f4dd1def6ae708` | exact match |
| `ketchup-gate-c-nav.exe` | `977141119f1c249e7e88fd90f5c59485cbf3d17f9a5bc50a3cd3e80f4d47e8f9` | exact match |

This establishes that the current build reproduces the binaries already measured on HP-DEV-01 rather than introducing a post-observation benchmark change.

## Repair

`scripts/windows/run-gate-c-hp-igpu-01.ps1` now:

1. validates every shared OCCT library in `third_party/occt-install-r0-v1` against the hashes in the R0-locked `artifacts/r0/occt-build-manifest.json`; and
2. after the locked release build but before the first formal sample, requires all three executable hashes to equal the immutable HP-DEV-01 provenance hashes.

The second check covers the compiled transitive Rust/C++/resource dependency closure. Any source, manifest, build-input, or compiler-output difference now stops the run before measurement. No threshold, corpus, hardware profile, measured product source, historical result, or done criterion changed.

## Verification

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Fresh release executable reproduction | **PASS** — all three hashes match HP-DEV-01 provenance |
| Frozen OCCT shared-library validation | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** |
| `git diff --check` | **PASS** |
| Qualification on the available desktop | **EXPECTED REJECTION** before fingerprint or measurement evidence creation |
| `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v11.json` | **ABSENT** |
| `artifacts/gate-c/hp-igpu-01-r0-v11-run-manifest.json` | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

## Done-check

Gate C remains open. The command criterion passes after this repair, but the report existence criterion still fails pending a qualifying physical HP-IGPU-01 notebook and its three core plus three NAV series. L1 #23 must remain active.
