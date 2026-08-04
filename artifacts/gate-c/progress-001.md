# Gate C Implementation Progress 001

**Status: implementation PASS; formal Gate C measurement not started**

## Delivered vertical slice

- `ketchup-interaction` owns CPU exact ray/box hit testing, stable occurrence/node/element identities, overlapping candidates, endpoint and midpoint snapping, deterministic scoring, and one shared authoritative geometry allocation for 10,000 occurrences.
- Preview state is ephemeral and cancellable. A stale relevant document edit rejects confirmation without mutation.
- Unambiguous Smart Push/Pull produces the same versioned UI `CommandBatch` used by the canonical core; ambiguous provenance requires a choice and emits no command.
- Preview and commit carry one matching command digest. The localized English action digest declares the exact source feature and old/new heights.
- `ketchup-app` is a narrow egui shell rendered through wgpu with only the Windows Direct3D 12 backend enabled. It displays a 3D exact-box representation, orbit/zoom navigation, CPU-exact viewport picking, height input, preview, confirm, and cancel.
- Every visible string is resolved from stable keys in `locales/en-US.ftl`; widget code contains no user-facing prose.

## Verified evidence

- Gate C interaction tests: 7 passed, 0 failed.
- App/unit shell tests: 4 passed, 0 failed.
- Direct3D 12 release startup smoke: window remained alive and responsive for five seconds.
- Release executable: `target/release/ketchup-app.exe`.
- Release executable SHA-256: `b6e5b814c30d446f9bc6406b286141cff206eb4b04b1f8f248bf5ff4297299b0`.
- R0 v8 lock SHA-256: `ad2f6ff3c89043d1491b02de1e0af390a3211ae4844cb37f32bacb3956b7c456`.
- `cargo deny check licenses sources`: passed.
- Immutable A0 `run-008`: GO under R0 v8, with 10,000/10,000 fuzz calls, 24/24 Guaranteed identity/history outcomes, zero silent invalid/wrong outcomes, and 3/3 STEP fixtures.

## Hardware status

The development machine matches frozen `HP-DEV-01`: Windows 10 Pro 10.0.19045, AMD Ryzen 9 5900X (12 cores / 24 threads), 63.9 GiB RAM, AMD Radeon RX 6800 XT, driver 32.0.21045.1000.

It does not satisfy mandatory `HP-IGPU-01`: it is not a 2023–2026 Windows 11 notebook with a 15–30 W mobile CPU and integrated Direct3D 12 GPU. No Gate C GO is claimed and the development workstation is not substituted for that required profile.

## Remaining Gate C work

1. Implement the frozen release measurement harness for `QC-C-NAV-01`, `QC-C-EDIT-01`, `QC-C-PICK-01`, and `QC-C-LONG-01`, preserving every raw sample and nearest-rank percentile.
2. Run three consecutive complete series on `HP-DEV-01`.
3. Record the exact first qualifying `HP-IGPU-01` fingerprint before its first observation, then run three complete series there.
4. Publish `artifacts/gate-c/report.md` only after both hardware profiles satisfy every frozen correctness and latency threshold.
