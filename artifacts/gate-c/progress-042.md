# Gate C Transfer-Residue Repair 042

**Status: the physical-notebook transfer preflight now rejects the complete reserved formal-evidence namespace before qualification; HP-IGPU-01 observation remains blocked on hardware availability**

- Diagnostic UTC: `2026-08-02T06:19:43.2645993Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- Repaired preflight: `scripts/windows/verify-gate-c-transfer.ps1`
- Repaired preflight SHA-256: `b1dd096d480bfd8f4af66ac3593c4618bb90192ba4185592497cd98be8642c54`

## Reproduced transfer flaw

The preflight introduced in progress 041 rejected only the fingerprint, attempt claim, run manifest, and terminal reports. It did not reject the frozen runner's reserved build logs, six metric files, twelve series logs, or `target/gate-c-r0-v12-hp-igpu-01`. A synthetic stale `hp-igpu-01-core-r0-v12-series-1.stdout.log` was placed in the transferred evidence namespace and the pre-repair preflight incorrectly returned PASS.

This was a consequential handoff defect rather than cosmetic hardening. On the physical notebook, the old preflight could authorize qualification and create the immutable notebook fingerprint, while the subsequent formal invocation would reject the stale reserved file or reused clean-build target before sealing the attempt. The operator would then have a fingerprinted but non-runnable handoff instead of the clean one-shot workspace promised by progress 041.

## Repair

The read-only preflight now requires absence of every path reserved by the frozen formal runner: the fingerprint, attempt claim, run manifest, two build logs, six metric files, twelve series logs, clean formal-build target, and three terminal report paths. The repair does not change a threshold, hardware profile, corpus, query class, consequence, measurement source, frozen runner, report validator, R0 lock, portable reference, or historical observation.

## Fresh validation

| Check | Result |
|---|---|
| Pre-repair stale stage-log reproduction | **REPRODUCED** - preflight incorrectly passed |
| Repaired preflight with the same stale stage-log path | **PASS** - failed closed before qualification |
| Repaired preflight with reused formal-build target | **PASS** - failed closed before qualification |
| Repaired preflight on the restored clean workspace | **PASS** |
| Repaired preflight PowerShell parser | **PASS** |
| R0 v12 preregistration and portable build-provenance validation | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |
| Synthetic residue cleanup | **PASS** - test log and target directory removed |
| Repaired preflight SHA-256 | **PASS** - `b1dd096d480bfd8f4af66ac3593c4618bb90192ba4185592497cd98be8642c54` |
| Preflight diff whitespace check | **PASS** |

## Done-check

The workspace-test condition passes. `artifacts/gate-c/report.md` remains absent, so the report-existence and `GO` conditions remain false. Gate C stays active under testable assumption A7.

## Next action

Copy the exact workspace and frozen OCCT tree to the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Independently verify the progress-042 preflight hash, run the repaired transfer preflight, run qualification-only and review the immutable fingerprint, then execute the three core and three NAV formal series exactly once and invoke `scripts/windows/write-gate-c-report.ps1 -WriteReport`.
