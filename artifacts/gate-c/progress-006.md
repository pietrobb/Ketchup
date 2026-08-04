# Gate C Implementation Progress 006

**Status: HP-IGPU-01 qualification and formal-run packet implemented; physical notebook still unavailable**

- Observation UTC: `2026-08-01T21:28:45Z`
- Active freeze: `r0-v11`
- R0 lock SHA-256: `d6c9edacd884a1b24a8fc6d42a14ad4bc25c248883faf7ba5c0d846977ae8de7`
- Runner: `scripts/windows/run-gate-c-hp-igpu-01.ps1`
- Runner SHA-256: `e85d24ca971d2386f1b2e4459f547d316c1594bfbfb84579726f181b7cf6e659`

## Progress artifact

The Windows runner now enforces the complete frozen `HP-IGPU-01` class before any Gate C observation. It captures stable system, product, BIOS, baseboard, enclosure, CPU, memory-module, GPU/driver, OS, battery-identity, display, scaling, AC-power, and power-profile evidence. It also requires explicit owner attestations for retail release evidence, nominal mobile CPU power, integrated-GPU identity and shared budget, OS patch state, disabled discrete GPUs, production driver, vendor-balanced profile, and a clean formal-measurement background.

A qualifying first invocation writes `hp-igpu-01-fingerprint-r0-v11.json` before measurement. Later invocations must match its stable machine/configuration digest and the runner hash; machine substitution, script changes, frozen source changes, existing raw evidence, or an existing run manifest fail closed. Dynamic battery charge is recorded but excluded from stable identity, while AC state remains mandatory.

With `-RunFormalMeasurements`, the runner revalidates R0 v11, requires a fresh background-state attestation, performs one locked release build, then sequentially runs three immutable core series and three immutable Direct3D 12 NAV series. It never overlaps formal measurements and creates a hash manifest only after all six runners return success. Raw failed evidence is preserved and never overwritten.

## Validation

- PowerShell parser validation passed.
- R0 v11 preregistration validation passed from inside the runner.
- The actual current HP-DEV-01 desktop was used as a negative qualification test and was rejected on notebook form factor, retail release evidence, Windows 11, mobile CPU class, integrated GPU, 16 GiB memory class, display configuration, AC/vendor-balanced state, and formal background confirmation.
- No `hp-igpu-01-*` fingerprint, series, or run-manifest artifact was created by the negative test.
- `cargo test --workspace --all-targets` passed, including A0 and formal Gate B.
- `git diff --check` passed for the complete working tree.

## Notebook execution sequence

1. On the first physically available qualifying notebook, invoke the runner without `-RunFormalMeasurements`, supplying the owner-attested release year, retail model evidence, nominal CPU power, integrated GPU name, shared GPU budget, and all confirmation switches. Review and retain the immutable fingerprint.
2. Invoke the same runner again with `-BackgroundStateConfirmed -RunFormalMeasurements`. The stored attestation is reused, but the machine/configuration digest and current background-state confirmation are rechecked.
3. Review all six raw outputs and `hp-igpu-01-r0-v11-run-manifest.json`; only then issue the Gate C GO or NO-GO report.

`artifacts/gate-c/report.md` remains absent. The mandatory physical `HP-IGPU-01` observations have not occurred, so Gate C remains open.
