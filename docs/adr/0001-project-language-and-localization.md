# ADR 0001: Project Language and Localization

- Status: Accepted
- Date: 2026-08-01
- Decision owner: Project owner

## Context

Ketchup is intended for an international user and contributor community. English is the common project language, but the desktop interface must support multiple languages without retrofitting localization after widgets, commands, errors, and file formats have already embedded English prose.

Localization concerns presentation. It must not make canonical documents, commands, schemas, geometry, or deterministic tests depend on a user's locale.

## Decision

### Project language

English is mandatory for:

- technical and user documentation;
- source code identifiers and comments;
- schemas, protocol fields, command names, error codes, and test names;
- commit messages and release notes;
- the complete default UI locale.

Historical local drafts may remain in another language as source evidence, but they are not committed as current project documentation. New normative decisions are written in English as ADRs.

### UI localization contract

The default locale is `en-US`. Every user-facing string must be resolved through stable localization keys and external locale resources from the first UI implementation.

A widget must not contain hard-coded user-facing prose. This includes labels, tooltips, menu items, dialogs, status messages, validation messages, units shown to users, accessibility text, and action-digest templates.

The implementation must provide:

1. a framework-independent localization service between view models and widgets;
2. a complete `en-US` resource set used as the fallback locale;
3. locale selection independent of document content;
4. parameterized messages rather than string concatenation;
5. plural-aware and number/unit-aware formatting in the presentation layer;
6. stable machine-readable error and diagnostic codes in core crates, mapped to localized prose only at presentation boundaries;
7. a visible marker and diagnostic for missing keys in development builds;
8. automated checks for missing English keys and a pseudo-locale test that exposes truncation and accidental hard-coded strings.

Fluent-compatible message resources (`.ftl`) are the baseline resource model unless measured implementation evidence justifies a replacement ADR. Localization keys use hierarchical, semantic names such as `menu.file.open`, not English sentences as identifiers.

### Locale-independent canonical state

Canonical commands and documents store semantic values, explicit units, IDs, expressions, and error codes—not localized labels or formatted numbers. Parsing user input may be locale-aware at the UI boundary, but it must produce the same canonical value before validation and commit.

Serialized files may store a preferred display locale as optional user metadata. That preference does not change geometry, command digests, deterministic results, or compatibility.

### Privacy and cloud language services

Translation or other language services must obey the project privacy baseline: model, document, workspace, prompt, and telemetry data stay local unless the user explicitly opts in for the operation or workspace. A locale choice is never implicit consent to cloud processing.

## Consequences

- The initial interface is English but is structurally ready for additional locale packs.
- UI work has a small upfront resource-key cost and avoids an expensive localization retrofit.
- Core and protocol errors need stable codes and structured parameters.
- Screenshots and visual layout tests must account for text expansion and right-to-left support even if the first shipped locales are left-to-right.
- A second real locale is not required for R0/A0, but Gate C must include a pseudo-locale and the FLP gate must include at least one second test locale.

## Enforcement

A change that introduces hard-coded user-facing prose in widgets, locale-dependent canonical state, or an incomplete English fallback fails review and tests. Exceptions require a new ADR; comments and developer-only assertions are not user-facing strings.