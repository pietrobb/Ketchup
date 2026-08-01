# Ketchup

Ketchup is an open-source, AI-native 2D/3D parametric modeler focused first on architecture, interiors, and furniture. The project targets fast direct interaction, exact geometry, deterministic canonical commands, and an optional AI assistant that cannot bypass validation or confirmation.

## Current phase

The project is in **R0 preregistration and toolchain validation**. Product implementation must follow the fail-able gate order documented in the [Architecture V3 Execution Contract](docs/architecture/EXECUTION_CONTRACT.md): R0, A0, A1, B, C, then First Lovable Product validation.

## Project baselines

- License: Apache-2.0
- Initial platform: Windows
- Core direction: Rust with Open CASCADE Technology behind a narrow C++ facade
- Rendering direction: `wgpu`
- Privacy: local-first; cloud AI only after explicit operation or workspace opt-in
- Capacity: project owner plus AI agents, with no guaranteed additional human FTE

## Language and localization

Technical documentation, code, identifiers, schemas, tests, and commit messages are English. The initial UI is English, but all user-facing copy must use localization keys and resources from the first widget; see [ADR 0001](docs/adr/0001-project-language-and-localization.md).

## Execution

The ordered autonomous work plan is recorded in the [R0-to-FLP mission manifest](docs/missions/ketchup-r0-to-flp_manifest.md). Current verified license and toolchain inputs are in the [R0 baseline](R0_LICENSE_AND_TOOLCHAIN_BASELINE.md).

No stable public API or native file-format compatibility is promised at this stage.
