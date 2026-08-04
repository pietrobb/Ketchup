# Gate C Windows Patch-Freeze Repair 016

**Status: HP-IGPU-01 now freezes the complete observable Windows servicing identity before the first Gate C observation**

- Observation UTC: `2026-08-02`
- Active freeze: `r0-v11`
- Testable assumption: `A7`
- Repaired runner SHA-256: `c191529ab8f50b6ad2852b9e9e0e0f23e4a1bd755892b0ec71ea390229193c9b`

## Diagnostic finding

The frozen `HP-IGPU-01` class requires Windows 11 to be fully patched before the first run and then frozen for the gate run. The runner captured the broad OS version and build number, but it did not include the Windows Update Build Revision (UBR), display version, or installed hotfix identities in the immutable machine-configuration digest. A cumulative update that preserved the broad build number could therefore occur between qualification and formal measurement without being detected.

This was a qualification-integrity gap, not a benchmark or threshold failure. No HP-IGPU-01 fingerprint, formal run manifest, or Gate C report exists, so the repair occurred before the first lower-reference observation.

## Repair

`scripts/windows/run-gate-c-hp-igpu-01.ps1` now captures and fingerprints:

1. the Windows display version;
2. the Windows Update Build Revision from the operating-system registry; and
3. the sorted unique `Win32_QuickFixEngineering` hotfix IDs.

These fields are part of `snapshot.os`, which is included in both the stored machine fingerprint and `machine_configuration_sha256`. A servicing-state change after qualification therefore causes the existing-fingerprint comparison to fail before formal sampling. The fully-patched operator attestation remains mandatory; the objective servicing identity supplements rather than replaces it.

No source benchmark, executable hash, threshold, corpus, hardware profile, historical result, or done criterion changed.

## Verification

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Windows servicing sources on the available host | **PASS** — display version `22H2`, UBR `6466`, 57 unique hotfix IDs |
| R0 v11 preregistration validator from the runner | **PASS** |
| HP-IGPU-01 qualification on the available desktop | **EXPECTED REJECTION** before fingerprint or measurement evidence creation |
| `cargo test --workspace --all-targets` | **PASS** — all workspace targets passed |
| `git diff --check` | **PASS** |
| `artifacts/gate-c/hp-igpu-01-fingerprint-r0-v11.json` | **ABSENT** |
| `artifacts/gate-c/hp-igpu-01-r0-v11-run-manifest.json` | **ABSENT** |
| `artifacts/gate-c/report.md` | **ABSENT** |

## Done-check

The workspace-test criterion passes. The report existence and `GO` criteria remain unmet because the mandatory physical `HP-IGPU-01` notebook has not been provided. L1 #23 remains active.

The next admissible action is to run the repaired qualification on the first qualifying 2023–2026 Windows 11 notebook, review and freeze its fingerprint, then execute three consecutive core series and three consecutive Direct3D 12 NAV series. Only those immutable observations can support `artifacts/gate-c/report.md` with `GO` or `NO-GO`.
