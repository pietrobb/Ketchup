# ADR 0003: Splash Screen and Version Display

- Status: Accepted
- Date: 2026-08-01
- Decision owner: Project owner

## Context

Ketchup has approved splash artwork at `pic/splash.png`. The artwork should identify the project on GitHub and may later provide immediate visual feedback while the desktop application initializes. A version rendered permanently into the source image would become stale and create duplicate version sources.

## Decision

The repository README displays `pic/splash.png` as project artwork. The original PNG remains the reusable source asset until an application asset pipeline is introduced.

When the desktop application gains a startup window, it may use this artwork under the following contract:

1. the version is rendered dynamically over the artwork from the build's single authoritative package version;
2. prerelease and development builds include their channel or short revision identifier where available;
3. startup status text uses localization keys and the same resource system required by ADR 0001;
4. the splash remains visible only while real initialization work is pending and closes when the first usable application window is ready;
5. progress is indeterminate unless the measured startup stages provide truthful completion data;
6. startup failure replaces the splash with an accessible localized error surface rather than leaving it frozen;
7. application startup must not depend on network access, telemetry, or cloud AI.

The source PNG does not contain a release number. Version formatting and placement belong to the application presentation layer and are covered by visual tests when the startup window is implemented.

## Consequences

- GitHub gets immediate visual identity without coupling the artwork to a release.
- Every build can show an accurate version without regenerating the PNG.
- Splash status and errors are localization-ready from their first implementation.
- Runtime integration is deferred until a desktop shell exists; this ADR does not add startup code during R0.
