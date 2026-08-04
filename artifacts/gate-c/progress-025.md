# Gate C Pre-Observation Binary-Reproducibility Diagnosis 025

**Status: a real HP-IGPU-01 formal-run portability defect was reproduced; Gate C remains unobserved and active**

- Diagnosis UTC date: `2026-08-02`
- Active freeze: `r0-v12`
- R0 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`
- HP-IGPU-01 runner SHA-256: `16ccaee667445f12b4d7ce6a47f66d094c1290ad3ec90ad41a6fb5fe147eb721`
- R0 v12 validator SHA-256: `a4e4b4225ca3eba95f1e6e91fad292fc511bffe6d7c272c228c930dfd3f3ac78`
- Testable assumption: `A7`
- Observation state: no HP-IGPU-01 fingerprint, attempt claim, stage log, metric series, run manifest, or Gate C report exists

## Reproduced defect

The formal runner executes a release build on the future notebook and then requires all three resulting executable SHA-256 values to equal the binaries previously observed on `HP-DEV-01`. A fresh-checkout notebook cannot reliably satisfy that requirement: clean Windows MSVC release builds are not byte-reproducible under the current build contract.

Two clean builds from the same frozen source, lock file, Rust toolchain, host, and repository path, differing only in `CARGO_TARGET_DIR`, produced different hashes for every executable:

| Executable | Clean target A SHA-256 | Clean target B SHA-256 | Equal |
|---|---|---|---|
| `ketchup-gate-c-core.exe` | `60a88001a0dcf599a416be502be290f9f213acdc11df95710eaae71d90c5c96a` | `d8d9c136b59dc61d5b05d90f53227f5582b849e6ac4c1f2881a29a62100d61c1` | **NO** |
| `ketchup-exact-worker.exe` | `74d52d584638125cf47f8687cd99b91d8ec251cf6bd36e7a621dc4f51d9198bc` | `e61b18972bb71a9bc171762908b4301740a06ebcea38880ceeb33c805f320b12` | **NO** |
| `ketchup-gate-c-nav.exe` | `2ade29673a47cf832f52936ab4c44dd03b019d8fa19426c27eb03b0a9735e721` | `9893298049228498fb895810ba56cffc233b12423c6a38daa19c69aea8ea544d` | **NO** |

A second diagnostic deleted and rebuilt one fixed target directory twice. This also produced different hashes on every executable, disproving the narrower hypothesis that only the target path caused the mismatch:

| Executable | Fixed-path clean run 1 SHA-256 | Fixed-path clean run 2 SHA-256 | Equal |
|---|---|---|---|
| `ketchup-gate-c-core.exe` | `3487f22817e2240f807078b4363b605920e44b3fe30b32ee84bb42181f8d2170` | `e2e650cb8ce3b700dc270e2ada75bcef1dc4fad14e51aa5de7882acf3318f293` | **NO** |
| `ketchup-exact-worker.exe` | `13d99752578311e9472c13e7750edf9b237502d5de6cf1b1eb6cafb32205946b` | `34ee4e83319c6a8ae24a9b45c32cff5448eb65b0ecb302e295e1ddb33f1d98b7` | **NO** |
| `ketchup-gate-c-nav.exe` | `ecc638f02d8b0ac64b0e2bcb50e95f33af10e1650265228434a8f81624cf78d2` | `af6d577c2e0c0606d7d9559533d0cdbb61e38427cd0cf53369ec2b7300fc6c67` | **NO** |

The current incremental `target/release` outputs still match the three hashes hard-coded in the runner, but that only proves the existing local build cache has retained the HP-DEV-01-observed files. It does not make those bytes reproducible on the required physical notebook. A clean notebook run would complete the release build and then become `INFRASTRUCTURE_INVALID` at `executable-verification` before any series, regardless of source correctness.

## Scope and evidence integrity

This diagnostic changed no runner, validator, threshold, corpus, hardware profile, oracle, product source, measurement source, expected outcome, or historical evidence. All temporary build directories were removed. No HP-IGPU-01 observation was started, so a focused pre-observation repair remains permissible without weakening or rewriting a result.

## Fresh verification

| Check | Result |
|---|---|
| Different-target clean-build comparison | **DEFECT REPRODUCED** — 0 of 3 executable hashes matched |
| Same-target clean-rebuild comparison | **DEFECT REPRODUCED** — 0 of 3 executable hashes matched |
| Current cached HP-DEV-01 executable hashes | **MATCH** the runner constants |
| PowerShell parser: runner and validator | **PASS** |
| R0 v12 preregistration validator and attempt-sealing self-test | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** |
| Frozen runner, validator, and R0 lock hashes | **MATCH** |
| HP-IGPU-01 fingerprint, claim, logs, metrics, and run manifest | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

The workspace-test portion of the L1 done-criteria passes. Report existence and `GO` remain unmet, so L1 #23 stays active.

## Required next action

Before acquiring or measuring the notebook, implement one focused pre-observation repair. The runner must use an auditable portable binary contract rather than asserting that a fresh local build reproduces the HP-DEV-01 PE bytes. The repair must preserve frozen source, dependencies, toolchain, build mode, thresholds, and all historical evidence; record the actual binaries used; retain fail-closed provenance checks; repin the runner and validator; and rerun the HP-DEV-01 reference series if the executable contract changes. Only after that repair is validated may the qualifying notebook be fingerprinted and measured.
