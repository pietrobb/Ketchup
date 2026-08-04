# Gate C Implementation Progress 007

**Status: HP-IGPU-01 qualification now fails closed on Direct3D 12 capability; physical notebook still unavailable**

- Observation UTC: `2026-08-01T21:49:09.3993701Z`
- Active freeze: `r0-v11`
- R0 lock SHA-256: `d6c9edacd884a1b24a8fc6d42a14ad4bc25c248883faf7ba5c0d846977ae8de7`
- Runner: `scripts/windows/run-gate-c-hp-igpu-01.ps1`
- Runner SHA-256: `1778568f392269a418b78b9fd35b4a5a7143d14a370b2ee0b575d430cd2c871b`

## Qualification correction

The frozen `HP-IGPU-01` class requires an integrated Direct3D 12 GPU. The prior runner required an observed integrated-GPU name, a production-driver confirmation, and confirmation that every discrete GPU was disabled, but it did not separately require the operator to confirm Direct3D 12 capability before writing the immutable pre-observation fingerprint.

The runner now requires `-Direct3D12Confirmed`. That value is stored in the operator attestation, participates in the immutable machine/configuration digest, and is revalidated on later invocations. A missing confirmation fails qualification before any fingerprint or Gate C measurement artifact is created. The formal NAV phase continues to force `WGPU_BACKEND=dx12`, so an adapter that cannot actually run the Direct3D 12 workload also fails during measurement while preserving any failed raw evidence.

No frozen threshold, corpus, hardware profile, query class, measurement source, or historical observation was changed. No qualifying `HP-IGPU-01` fingerprint exists, so updating the unobserved acquisition runner does not rewrite or invalidate Gate C evidence.

## Validation

- PowerShell parser validation passed.
- R0 v11 preregistration validation passed from inside the runner.
- A negative qualification invocation with every GPU attestation except `-Direct3D12Confirmed` was rejected with the required Direct3D 12 failure.
- The negative invocation created no `hp-igpu-01-fingerprint-r0-v11.json` or other notebook evidence.
- `cargo test --workspace --all-targets` passed, including immutable A0 run-011 coverage and formal Gate B.
- `git diff --check` passed.
- `artifacts/gate-c/report.md` remains absent.

## Remaining physical step

On the first qualifying notebook, invoke the runner in qualification-only mode with all prior owner attestations plus `-Direct3D12Confirmed`. Review the resulting immutable fingerprint, then invoke the unchanged runner with `-BackgroundStateConfirmed -RunFormalMeasurements`. Gate C may receive a GO or NO-GO report only after the three core and three NAV series have completed and their evidence has been reviewed.
