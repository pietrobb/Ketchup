# Gate C Portable Build-Provenance Repair 026

**Status: the pre-observation cross-machine binary defect is repaired and frozen; clean-build core reference evidence passes; Gate C remains active and unobserved on HP-IGPU-01**

- Repair UTC date: `2026-08-02`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- Repaired HP-IGPU-01 runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- Repaired R0 v12 validator SHA-256: `2efd7ab90ff199c2cd9669fbb603af6ba1db58b1ef264e4d126baed5564c0c56`
- Portable build-input tree SHA-256: `6dc2be8e1cfe992247d2946853c77977915ba249930437b6797f0b053d65b3b6` across 29 files
- Testable assumption: `A7`
- Observation state: no HP-IGPU-01 fingerprint, attempt claim, stage metric, run manifest, or Gate C report exists

## Repair

The runner no longer requires a clean notebook build to reproduce the byte hashes of cached HP-DEV-01 PE files. Before qualification it now fails closed unless all of the following match the frozen contract:

1. the R0 v12 lock and four direct measurement-source hashes;
2. one canonical digest covering `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, every file below `crates/`, and every locale resource below `locales/`;
3. exact Rust 1.97.0, Cargo 1.97.0, MSVC compiler, and MSVC linker executable hashes recorded by the frozen OCCT manifest;
4. the complete 7,400-file OCCT install-tree fingerprint, not only its runtime DLL subset; and
5. the exact locked release-build arguments and Windows MSVC target.

The qualification fingerprint and terminal attempt manifest preserve this complete build provenance. Formal measurement builds in a dedicated target directory that must not already exist, invokes the exact pinned tools, records the actual hashes of all three executables, and then runs those exact files. A stale cache therefore cannot satisfy the contract, while legitimate MSVC linker nondeterminism no longer invalidates a source-identical notebook build.

The runner retains atomic qualification, atomic attempt claiming, immutable per-stage stdout/stderr, deterministic `PASS`/`FAIL`/`INFRASTRUCTURE_INVALID` classification, terminal manifest sealing, and overwrite refusal. The validator explicitly rejects the removed HP-DEV PE-equality contract and pins the repaired runner hash.

## Clean-build reference evidence

A new isolated target was absent before execution. The pinned toolchain completed the exact locked release build in 36.84 seconds. Its executable hashes differ from the earlier cached HP-DEV binaries, reproducing the expected PE nondeterminism, but all three fresh core series passed with the same `fnv1a64:fef50903b056d7c8` result fingerprint, 100 percent action-digest agreement, zero wrong identities, and zero committed data loss.

| Series | Edit p95 ms | Pick/snap p95 ms | Navigation block max ms | Cancel p95 ms | Result |
|---|---:|---:|---:|---:|---|
| 1 | 2.4312 | 2.4307 | 2.8044 | 1.0795 | **PASS** |
| 2 | 2.4407 | 2.4377 | 2.8452 | 0.9561 | **PASS** |
| 3 | 2.2790 | 2.4734 | 3.2787 | 1.1023 | **PASS** |

The immutable summary is `artifacts/gate-c/hp-dev-01-portable-core-r0-v12-provenance.json`.

A fresh NAV reference was also attempted from the clean binary. Its preregistered duration is 30 repetitions of 10 seconds warm-up plus 30 seconds measurement, or 1,200 seconds per series. The orchestration command was bounded to 600 seconds and terminated before series 1 could complete. It produced empty stdout/stderr logs and no metric JSON; no NAV process remains. This is not a performance result and does not replace or alter the existing passing r0-v12 NAV evidence. A later tick must execute each clean NAV series with a greater-than-1,200-second process budget and non-overlapping evidence names.

## Fresh verification

| Check | Result |
|---|---|
| Runner PowerShell parser | **PASS** |
| Validator PowerShell parser | **PASS** |
| Portable provenance plus attempt-sealing self-tests | **PASS** |
| R0 v12 preregistration validator | **PASS** |
| Clean isolated locked release build | **PASS** |
| Three clean-build HP-DEV-01 core reference series | **PASS** |
| Clean-build NAV replacement series | **PENDING** — one bounded diagnostic attempt ended before the 1,200-second harness duration |
| `cargo test --workspace --all-targets` | **PASS** |
| `git diff --check` | **PASS** |
| HP-IGPU-01 evidence | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

The workspace-test portion of the L1 done-criteria passes. Report existence and `GO` remain unmet, so L1 #23 stays active.

## Next action

Run three sequential clean-build HP-DEV-01 NAV reference series with a per-process budget above 1,200 seconds and seal their provenance without changing the repaired runner. Then provide the first qualifying 2023-2026 Windows 11 integrated-GPU notebook, capture and review its immutable fingerprint, execute the six formal series exactly once, and issue the evidence-based Gate C report.
