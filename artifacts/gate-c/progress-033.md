# Gate C Qualified-Fingerprint Closure Repair 033

**Status: a pre-observation false-qualification path in the closure validator is repaired and frozen; the physical-notebook blocker remains**

- Repair observation UTC: `2026-08-02T04:51:13.8191941Z`
- Active measurement freeze: `r0-v12`
- Testable assumption: `A7`
- Repaired report validator: `scripts/windows/write-gate-c-report.ps1`
- Repaired report validator SHA-256: `6f1dc6fe5919816ac85c8a351dbeafb94fd0d4ea92ebdea312688c8fddb4381e`
- Superseded pre-observation report-validator SHA-256: `6cea6c56d9cfbe0fa913b876ea54bbcb5b1a0ec1c125ad6829699e337f9c7d4c`
- Frozen runner SHA-256: `cf8bb2ef587e0925ecfafa05908179b30fae53a4e738a87af6162c1f8536d164`
- R0 v12 validator SHA-256: `2efd7ab90ff199c2cd9669fbb603af6ba1db58b1ef264e4d126baed5564c0c56`
- R0 v12 lock SHA-256: `01ee1e30d4a9026f674ee3ee9fa1dde965294b97b691b7870fab49c782b96176`

## Reproduced gap

The progress-032 closure validator required `qualification_decision: PASS` and the expected runner, lock, and build-input hashes, but it did not independently re-evaluate the serialized fingerprint against the frozen HP-IGPU-01 hardware profile. A synthetic fingerprint could therefore claim qualification without proving the required notebook chassis and battery, 2023-2026 retail model, Windows 11 build, mobile CPU, sole operational integrated GPU, 16 GiB memory envelope, 1920x1080 60 Hz display at 96 DPI, AC power, balanced profile, or clean background state. The PASS metric path remained independently checked, but the required hardware identity boundary was incomplete.

This was found and repaired before any HP-IGPU-01 fingerprint or formal observation existed. No threshold, corpus, hardware profile, measurement source, runner, R0 lock, HP-DEV observation, or historical gate artifact was changed.

## Repair

The closure validator now independently enforces every objective and attested HP-IGPU-01 qualification criterion serialized by the frozen runner. It also reconstructs the runner's canonical machine-configuration object from the fingerprint and verifies its SHA-256 digest, so post-capture edits to the machine or attestation block fail closed.

The report validator remains post-observation-only: it still cannot create `artifacts/gate-c/report.md` without an immutable qualifying fingerprint, exclusive attempt claim, and terminal run manifest. The repaired validator hash above supersedes the progress-032 hash while the measurement state is still `not_started`.

## Validation

| Check | Result |
|---|---|
| PowerShell parser | **PASS** |
| Synthetic desktop fingerprint falsely claiming `PASS` | **PASS** - rejected as not an objectively identified physical notebook |
| Synthetic qualifying notebook fingerprint with the runner-equivalent configuration digest | **PASS** - accepted through qualification and rejected later at the intentionally invalid attempt schema |
| Qualified-looking fingerprint with a changed configuration digest | **PASS** - rejected before attempt processing |
| Complete qualified synthetic seven-stage PASS set | **PASS** - generated `**Decision: GO**` only inside an isolated temporary evidence directory |
| Temporary report overwrite boundary | **PASS** - the test used exclusive report creation and removed the entire temporary directory afterward |
| Real repository without HP-IGPU-01 evidence | **PASS** - failed closed; no report was created |
| Temporary validator evidence after tests | **PASS** - no `.tmp-report-validator-*` path remains |
| R0 v12 preregistration validator | **PASS** |
| `cargo test --workspace --all-targets` | **PASS** - all 32 tests |
| `git diff --check` | **PASS** |

Synthetic evidence exercised validator control flow only. It was never placed at the real HP-IGPU-01 paths, never represented a physical observation, and was deleted after each test.

## Gate status

The done-check remains false. The real `hp-igpu-01-fingerprint-r0-v12.json`, attempt claim, run manifest, six notebook metrics, and `artifacts/gate-c/report.md` are absent. No GO, notebook identity, or physical measurement was fabricated.

## Next action

Provide the first qualifying physical 2023-2026 Windows 11 integrated-GPU notebook. Run qualification-only through the frozen runner and review the exclusively created fingerprint, then run the three core and three NAV formal series exactly once. Finally invoke the repaired `scripts/windows/write-gate-c-report.ps1 -WriteReport`; it will issue GO, NO-GO, or infrastructure-invalid only after independently revalidating the physical-notebook fingerprint and sealed measurement evidence.
