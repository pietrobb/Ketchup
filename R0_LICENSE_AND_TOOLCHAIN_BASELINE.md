# Ketchup R0 License and Toolchain Baseline

- Status date: 2026-08-01
- Purpose: verified inputs before A0; not legal advice
- Architecture contract: `docs/architecture/EXECUTION_CONTRACT.md`

## 1. Result

- No known license blocker was found for the proposed open-source stack.
- Ketchup is licensed under Apache-2.0.
- The PoC is Windows-first.
- Open CASCADE Technology (OCCT) may be combined with separately licensed Ketchup code.
- The default distribution model is unmodified OCCT 8.0.1 as replaceable shared libraries behind Ketchup's narrow C++ facade.
- A process or worker boundary does not remove license obligations.
- A0 builds OCCT from source with only required modules and optional integrations disabled. It does not use an aggregated third-party binary bundle.
- Formal legal review is not planned. The internal OCCT compliance checklist remains mandatory before public binary distribution.

## 2. Verified dependency baseline

| Component | R0 version/status | License | Decision |
|---|---:|---|---|
| Open CASCADE Technology | 8.0.1, tag `V8.0.1`, commit `b8f597c677811d1f9f4d8a97f5ae2825c0353a42` | LGPL-2.1 with Open CASCADE Exception 1.0; commercial alternative | Source build, shared libraries, no local OCCT modifications for A0 |
| Rust toolchain | 1.97.0 stable | MIT or Apache-2.0, depending on component | Pin with `rust-toolchain.toml` when the workspace is initialized |
| `cxx` bridge | Pin in the first `Cargo.lock` | MIT OR Apache-2.0 | Allowed; API remains inside Ketchup's facade |
| `egui` | 0.35.0 | MIT OR Apache-2.0 | PoC candidate, not a long-term commitment |
| `wgpu` | 29.0.4 with egui 0.35.0 | MIT OR Apache-2.0 | Pin 29.0.4; egui 0.35.0 declares `wgpu = "29.0"` |
| Wasmtime | Deferred; release 47.0.3 verified at R0 | Apache-2.0 | Excluded from R0/A0; recheck before a plugin pilot |
| CMake | Local 4.2.1 | BSD-3-Clause | Sufficient; freeze final fingerprint after clean configure |
| MSVC Build Tools | Local VS 2022 17.5.33530.505 | Proprietary build tool | Not redistributed; update decision required before A0 |

`wgpu` 30.0.0 may be newer as a standalone release, but it is not the correct pin for egui 0.35.0. The PoC uses one compatible render stack.

## 3. OCCT license and distribution contract

### 3.1 Permitted model

LGPL-2.1 permits linking a work that uses the library and distributing the result under separate terms when LGPL obligations are met. The Open CASCADE Exception also permits object code containing material from OCCT headers under separate terms when use of OCCT is prominently disclosed.

OCCT's license does not automatically require Ketchup's separate source code to use LGPL, does not restrict running OCCT, and does not apply to ordinary CAD output created by users. Distribution of binaries containing or accompanied by OCCT has specific obligations.

### 3.2 Mandatory internal checklist before public binary distribution

A Ketchup distribution must:

1. prominently state that it uses Open CASCADE Technology;
2. include the complete LGPL-2.1 and Open CASCADE Exception texts;
3. preserve OCCT copyright, license, and warranty notices;
4. make the exactly corresponding OCCT source, including any Ketchup modifications, available;
5. keep dynamically linked OCCT libraries genuinely replaceable by a compatible user build;
6. not prohibit reverse engineering used to debug a user's OCCT modification;
7. record source tag and commit, compiler, Windows SDK, CMake options, DLL list, and checksums;
8. keep Ketchup facade changes separate from OCCT source changes; any modified OCCT files require notices and LGPL-available source.

Expected future package content:

- `THIRD_PARTY_NOTICES`;
- `licenses/OCCT-LGPL-2.1.txt`;
- `licenses/OCCT-LGPL-EXCEPTION-1.0.txt`;
- the exact OCCT source archive or equivalent access from the same distribution location;
- a reproducible build manifest;
- replaceable OCCT DLLs outside the main Ketchup executable.

### 3.3 Prohibited shortcuts

- Static linking is not the default. It requires satisfying LGPL relinking duties and a separate explicit decision.
- A worker process is not a way around LGPL.
- A transient upstream URL is not by itself a durable corresponding-source plan.
- Do not use an official combined Windows package without a separate SBOM and license review for every bundled dependency.
- Do not enable optional OCCT modules merely because CMake discovers them.

## 4. Minimal OCCT build for A0

A0 needs Foundation Classes, Modeling Data, Modeling Algorithms, and the portion of Data Exchange required by the frozen external STEP corpus. It does not need OCCT Visualization, DRAW, VTK, FreeImage, FFmpeg, OpenVR, Draco, Qt, or Tcl/Tk.

Build policy:

- `BUILD_LIBRARY_TYPE=Shared`;
- C++17;
- Visual Studio 2022 x64;
- disable Visualization and DRAW if clean configure confirms that A0/Data Exchange do not require them;
- default every optional `USE_*` integration to `OFF` and enable one only through a separate ADR;
- do not use OCCT's OpenGL renderer because Ketchup uses `wgpu`;
- do not modify OCCT source during the first A0 run;
- record the CMake cache, compiler version, Windows SDK, DLL list, and SHA-256 values.

OCCT documentation requires C++17 and Visual Studio 2019 or later and prefers Visual Studio 2022. OCCT 8.0.1 is the first 8.0 maintenance release and retains the C++17/API/ABI baseline of 8.0.0p1.

## 5. Ketchup license decision

**Decision O-01: Apache-2.0.** The priority is maximum freedom for free, commercial, internal, and closed derivative use while retaining explicit patent terms and broad compatibility with the Rust ecosystem and future integrations.

The owner accepts that third parties may distribute closed derivatives when they comply with Apache-2.0 notices and terms. MPL-2.0 would instead require source availability for modifications to existing MPL-covered files. GPL and AGPL are not selected because they would impose stronger conditions on the application and plugin ecosystem.

## 6. Verified local baseline

- OS: Windows 10 Pro 10.0.19045
- CPU: AMD Ryzen 9 5900X, 12 cores / 24 threads
- RAM: 63.9 GB
- GPU: AMD Radeon RX 6800 XT
- Storage: Samsung SSD 990 PRO 2 TB and GIGABYTE GP-AG42TB 2 TB
- Installed Rust/Cargo: 1.94.1
- Installed CMake: 4.2.1
- Installed Visual Studio Build Tools 2022: 17.5.33530.505

This machine is a high-performance B/C profile, not the only performance profile. Gate C also requires a mainstream integrated-GPU notebook.

## 7. Accepted operating decisions

- Platform: Windows-first.
- Capacity: project owner plus AI agents, with no guaranteed additional human FTE.
- Privacy: local-first. Model, document, workspace, prompt, and telemetry data are not sent to cloud AI unless the user explicitly opts in for the operation or workspace.
- Project language: technical documentation, code, identifiers, schemas, tests, and commit messages are English.
- UI language: `en-US` is the complete default and fallback locale. Every widget resolves user-facing copy through localization keys/resources; hard-coded user-facing prose is prohibited.
- Public compatibility: no stable API or native-file-format promise yet.
- Rust dependency license scan: run `cargo-deny` for every frozen `Cargo.lock`.
- Release SBOM: CycloneDX or SPDX plus a binary manifest.
- Default allowed licenses: Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, and individually reviewed LGPL with replaceable dynamic linking.
- GPL, AGPL, SSPL, unclear, and custom licenses are blocked pending a dedicated ADR and review.

Localization details are binding in `docs/adr/0001-project-language-and-localization.md`.

## 8. Open work before A0

1. Pin or install Rust 1.97.0 for the project.
2. Decide and perform any required update from local MSVC 17.5 to a supported VS 2022 branch before building OCCT.
3. Produce a clean, reproducible OCCT build fingerprint.
4. Add a provenance- and license-safe external STEP corpus. Random internet CAD files and undocumented OCCT test data are not acceptable.
5. Freeze fixed, generative, mutation, and adversarial corpora plus the threshold file before observing A0 results.
6. Define a second B/C reference profile using a mainstream integrated-GPU notebook.

The connected public repository is `pietrobb/Ketchup`; local `main` tracks `origin/main`. This first project baseline commit does not itself complete R0.

## 9. Primary sources

- OCCT releases: https://dev.opencascade.org/release
- OCCT 8.0.1 release and commit: https://github.com/Open-Cascade-SAS/OCCT/releases/tag/V8.0.1
- OCCT licensing: https://dev.opencascade.org/resources/licensing
- OCCT 8.0.1 license declaration: https://github.com/Open-Cascade-SAS/OCCT/tree/V8.0.1
- OCCT build requirements and options: https://dev.opencascade.org/doc/overview/html/build_upgrade__building_occt.html
- Rust 1.97.0 release: https://blog.rust-lang.org/2026/07/09/Rust-1.97.0/
- Rust licensing: https://rust-lang.org/policies/licenses/
- wgpu repository: https://github.com/gfx-rs/wgpu
- wgpu releases: https://github.com/gfx-rs/wgpu/releases
- egui 0.35.0 manifest: https://github.com/emilk/egui/blob/0.35.0/Cargo.toml
- egui releases: https://github.com/emilk/egui/releases
- CXX bridge: https://github.com/dtolnay/cxx
- Wasmtime repository: https://github.com/bytecodealliance/wasmtime
- Wasmtime releases: https://github.com/bytecodealliance/wasmtime/releases
- Microsoft Windows Rust/MSVC setup: https://learn.microsoft.com/en-us/windows/dev-environment/rust/setup

## 10. R0 status

**License research pass:** conditionally successful; no known blocker.

**A0 must not start yet:** the reproducible OCCT build fingerprint, frozen corpus, and frozen threshold file are still missing.