# ADR 0007: Windows x86-64 First Release

- Status: Accepted — project owner, 2026-08-09
- Date: 2026-08-09
- Decision owner: Project owner
- Accountable approver: Project owner
- Resolves: V4-O08
- Evidence: `scripts/windows/build-release-candidate.ps1`, `scripts/windows/test-release-candidate.ps1`; the release dialog evidence scripts (run-release-dialog-evidence.ps1, test-release-dialog-evidence.ps1) were retired with gate governance in commit `0794b8c` (ADR 0008).

## Context

V4-O08 required an explicit choice between a Windows-only first release and parallel desktop-platform support before release packaging could be frozen. The implemented and tested release path is Windows x86-64: it builds the desktop application and exact worker, packages the pinned co-located OCCT runtime, launches from a foreign working directory, and verifies live DLL provenance. Parallel macOS or Linux release work would add packaging, native-dialog, graphics-driver, accessibility, and hardware-certification obligations before the basic product workflow has been physically proven.

## Decision

1. The first Ketchup release targets Windows x86-64 only.
2. M19 may freeze and certify the Windows x86-64 package without a parallel macOS or Linux package.
3. macOS, Linux, and other platforms are deferred. Supporting one later requires a separate explicit scope decision and platform-specific packaging, native-dialog, accessibility, graphics, and hardware evidence.
4. Platform-neutral core architecture remains desirable, but it must not delay proof that the Windows product works end to end.
5. This decision resolves V4-O08 only. It does not claim that the current build is release-ready or that the physical New/Open/Save/Save As, failure-continuity, canonical-task, or current-tree hardware gates have passed.

## Consequences

- Release manifests may record `windows-x86_64-first-release` and cite this ADR instead of claiming that V4-O08 is open.
- The immediate release focus is observable product operation on Windows, not parallel-platform infrastructure.
- G19-02 still requires a named physical workflow run on release hardware.
- G19-03 and G19-04 remain required before M19 release readiness can close.
- No compatibility or future-support promise is made for macOS or Linux.

## Acceptance

Accepted by the project owner on 2026-08-09 with the explicit direction that Windows is sufficient for the first release and that proving the product actually works takes priority over other desktop platforms.

## Addendum 2026-10-11: macOS development build

The project owner approved a macOS (Apple Silicon) development build that does not gate the Windows release. `scripts/macos/build-occt.sh` builds the same pinned OCCT 8.0.1 module set and `scripts/macos/build-app.sh` packages `Kečup.app` with the exact worker and OCCT libraries; the app renders with Metal. The first release still targets Windows x86-64 only, and this build carries no release, signing, notarization or hardware evidence. As on Windows, AI clients drive the app through MCP (`ketchup-app --mcp`).
