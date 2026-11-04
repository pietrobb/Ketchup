# Review 2026-11-04: status of the findings

Branch `r19-review1104` (`9bb97bf0` + review `a6f131f4`). Every finding of
`review-2026-11-04.md` is listed here as fixed (with its commit) or deferred (with the
reason). Each code fix began with a test that failed before it.

## P1

| Finding | Status |
|---|---|
| PG-140: bracing not anchored to anything still braces | Fixed, b061b280: bracing counts only on a fastened path to a grounded part, a support or a declared diaphragm; a board on two free beams braces neither |
| PG-141: friction braces the whole length | Fixed, b061b280 and 98bdc98e: a part along the member needs a fastener on it, and a part anchored only in line with the member does not hold it sideways |
| PG-142: fastener positions ignored | Fixed, b061b280: with `fasteners`, bracing holds at their positions only |
| PG-143: an unrelated attribute inflates the cutting plan | Fixed, 91042649 and 23e5c408: only the grain, or the attribute a sheet stock names (`material:axis`), ties a blank to a profile axis; e71f93ed compares the 46° rafter's section within a tolerance, since Linux and macOS compute 140.00000000000023 where Windows computes 140 |
| SEC-140: Connect AI breaks Codex's `config.toml` | Fixed, f00f37d1: edited through `toml_edit`; a file that does not parse is left alone |
| CD-140: a deletion can remove another entity after the source changed | Fixed, c81539db and b5c0342b: removed objects and the children of edited ones are checked in place, an entity lying elsewhere refuses the export, and entities changed or added since the import are counted; 8615fa09 brings the DXF rewrite fuzz target to the new export result. **Deferred:** storing the source identity (size, `$HANDSEED`, entity count) at import needs a change of the underlay format with a persistence schema audit; until then a file that changed is detected by its entities, and entities no line was read from are not compared |
| REL-140: README promises the runtime 0.3.1 lacks | Fixed, eb40ffa9 (README, CHANGELOG known issue and Unreleased) |

## P2

| Finding | Status |
|---|---|
| SEC-141: writing a configuration weakens its permissions and replaces a symlink | Fixed, 6305ee83: permissions and ACL kept, symlink target written, one `.bak`, fsync before rename |
| SEC-142: the JSON rewrite loses settings and misses the project scope | Fixed, 859e604e |
| MCP-140: an MCP PDF or sheet export replaces any file | Fixed, e7603ef2: only the file the window last wrote there, or `replace: true` |
| MCP-141: takeover or Disconnect leaves the AI's sheet settings | Fixed, cea6c9bd; its regression (connection lost before the last frame) fixed in 3b91a2d0 |
| MCP-142: queued calls run after the client left | Fixed, 6cb7ed59: one worker in order, ping, `tools/list` and cancellation answered at once |
| MF-140: inconsistent decimal separator on cs/sk sheets | Fixed, 0c4956b3 |
| MF-141: ±0,000 wrong with a basement, some levels unmarked | Fixed, 7fffa72c (with its build repair 157efab8), 35a337d1 and 2e568335: ±0,000 is the ground floor or a sheet setting, every level a person stands on is marked (galleries and landings by their true width), and the bottom of the model |
| UI-140: File > New resets window settings | Fixed, 17340c3f |
| UI-141: Open Recent blocks on network paths | Fixed, 807eef9c |
| UI-142: Open Recent and the language not kept on macOS | Fixed, a8c92ed0 and 1ef2ea2a |
| UI-143: the Program tab lays out every line each frame | Fixed, df8b2927 |
| UI-144: a typed Move after locking an axis mid-drag has no direction | Fixed, 7335faff |
| PG-144: a logical group grounds its parts for statics | Fixed, b061b280 |
| UL-140, UL-141, UL-142: DXF encoding regressions | Fixed, 6e8937c5: one encoding decision for import, `text_encoding` and rewrite |
| REL-141: notices omit the Microsoft runtime | Fixed, 0391c1c6 |
| REL-142: OCCT `TK*.dll` unsigned | Fixed, 1df83b42 |
| REL-143: the release check misses struct fields | Fixed, a3a22ef9: each released reader opens and saves every file and HEAD compares the result |
| TST-140: the error ratchet is bypassed | Fixed, 540ebe34: `map_err_ignore`, `let_underscore_must_use` and `let_underscore_untyped` denied in every crate, every site fixed; 66d98ce8 makes the plugin bridge tests do their slow setup before connecting, so a loaded machine no longer outlasts the 2 s token wait |

## Findings of 11-02 the review found partly done

| Finding | Status |
|---|---|
| MCP-101: the deferral reason was wrong (FEA runs on the UI thread) | Fixed, 2f856263: the FEA review runs off the UI thread and can be cancelled |
| SEC-101: FUSE types missing on Linux, the check is lexical | Fixed, f7763737: every FUSE file system without a block device is remote, and links are resolved before the check |
| PG-123/108 rest: the material takeoff sorts by `blank_size()` | Fixed, 53b165b0 |
| WB-42 rest: `shelves=1e300` | Fixed, 45c67833 |
| MF-121: a worktop with a sink opening is refused | Fixed, fbcf3244 |
| MF-123: bare `ExportBlocked` refusals | Fixed, fbcf3244 |
| CD-120: a moved INSERT the new reader reads differently is refused; a DXF without units loses its bindings; CHANGELOG overstates | Units fixed, c81539db; the CHANGELOG names the INSERT case. **Deferred:** binding a moved INSERT read differently needs the block's own entities matched across reader versions; it is refused, so nothing is lost |
| UO-51: a binding name can shadow a builtin | Fixed, 84eb4657 |
| UI-101 rest: Open Recent and a delayed Open ask "discard changes?" before checking the file is a document | **Deferred:** the check reads the file header, which must not run on the UI thread (UI-141); it belongs with moving Open off the UI thread |
| EX-40: memory of jobs together is not told to users | Documented in the README (this branch) |
| EX-62: face references under tolerance belong in the README | Documented in the README (this branch) |
| CI-62: the macOS memory-limit test runs on no macOS runner | The test is fixed (bbbb7253). **Deferred:** running process-wrap's tests on the macOS runner is left to the macOS instance, which owns that job |
| TST-121 | See TST-140 |

## P3

| Area | Item | Status |
|---|---|---|
| AI and MCP | `%VARIABLES%` differs from Supervisor | Fixed, c45164c9 |
| | cancel after an edit was applied | Fixed, 77ca82d0 |
| | cancel before the id is registered | Fixed, 6cb7ed59 |
| | stale `started` | Fixed, 77ca82d0 |
| | `try_clone` failure blamed on the window | Fixed, 77ca82d0 |
| | `running_programs` without `GetLastError`, `.ok()?` in `mcp_setup.rs`, `connect_ai.rs` | Fixed, c45164c9 |
| | module-wide `#![allow(unsafe_code)]` | Fixed, c45164c9: allowed per item |
| Program | takeoff `blank_size()`, `shelves=1e300` | See above |
| | loft shell against the whole part | Fixed, 9a95a5c6 |
| | membrane area in the takeoff | Fixed, 4e1e1046 |
| | vertical `dim()` lost at 1e-5 mm, repeated on opposite elevations | Fixed, 75a3c4f9 |
| Drawings and manufacturing | message hides the backup; dropped roll back errors; Save As copies share a set; a crash between sheets and list | Fixed, c561acce |
| | MECH-4 drops the client's stamp | Fixed, a0b69fe4 |
| | underlay colour 7 on `PEN-0_13` | Fixed, 8702eb3b |
| | unreadable underlay dropped through `.ok()` | Fixed, d2105c5a |
| | underlay loses texts, fills and line types | **Deferred:** drawing texts, hatches and line types of an underlay in plans is a new feature, not a repair |
| | MECH-9 names the definition | Fixed, 189e6e97 |
| GUI | dock tabs without a focus frame or shortcut | **Deferred:** keyboard access to the dock is accessibility work of its own; nothing is lost without it |
| | badge `99` instead of `99+` | Fixed, 06f36f70 |
| | two windows do not share the tab and the recent list | **Deferred:** needs the settings store read again on every change, structural proposal 3 |
| | an offline entry is dropped when clicked | Fixed, 06f36f70 |
| | files opened over MCP join the user's list | Fixed, 06f36f70: documented in the menu, the MCP tool and the README |
| | selection card writes a point; `-0` | Fixed, 06f36f70 |
| | `save_view` ignores the result; sharing default depends on history | Fixed, 06f36f70 |
| | CHANGELOG misses #15, #17, #18, #20 | Fixed, eb40ffa9 |
| Underlays | DXF without units in another unit loses bindings | Fixed, c81539db |
| | binding name shadows a builtin or `A_POINTS` | Fixed, 84eb4657 |
| | `\U+XXXX` where the code page has the character; emoji as a surrogate pair | Fixed, f84934a0 |
| Platform, CI, release | PE imports never checked against a real System32; `ext-ms-*` taken as present | **Deferred:** needs a clean Windows image without the Visual C++ runtime as a CI runner, which GitHub does not offer; the package is checked against its own payload and the runtime it carries |
| | runtime from the newest redistributable | Fixed, 6f6819be: from the toolset that linked the programs |
| | old `build-release-candidate.ps1` | Fixed, 6cda1bef (the scripts' removal landed in f7763737) |
| | clippy without features in no job | Fixed, 71e3e47b |
| | `connect_ai_e2e.rs:169` budget through a helper `check_test_timing` cannot see | Fixed, 7b1788e3 (see its git note) |
| | macOS memory test with compressible data, `_ => return` | Fixed, bbbb7253 |
| | `CFBundleVersion` 0.3.2-dev, `versionCode` equal to 0.3.2 | Fixed, e4ea558a |
| | `viewer/web/tests`, wasm, APK and installer build in no job | **Deferred:** each needs its own toolchain on a runner (wasm-pack, Android NDK, Inno Setup with signing); a CI branch of its own |
| | `fuzz/Cargo.lock` and `third_party/process-wrap` in no ratchet | Fixed, 71e3e47b: `cargo deny` checks their advisories, bans and sources |
| | the format 108 test checks only `DIGEST_108` | Covered by REL-143, a3a22ef9: every released reader opens and saves the files |
| | CHANGELOG Unreleased incomplete; "0.3.1 and older refuse 108" wrong for 0.1.0 | Fixed, eb40ffa9 and this branch's CHANGELOG commit |

## Structural proposals

1. Bracing as an explicit model: the implicit part is done (b061b280, 98bdc98e). An
   explicit `brace()` is not added; PG-124 stays `not_verified`, on the safe side.
2. One way to edit AI client configuration: done (f00f37d1, 6305ee83, 859e604e).
3. Splitting `KetchupApp` into window state and document session: not done. UI-140 keeps
   the fields and UI-142 adds one settings file for every platform.
4. One export target policy: MCP-140 and c561acce share "what we wrote". CNC and the plan
   DXF keep their own checks.
5. One job lifecycle and an ordered MCP queue: done (cea6c9bd, 6cb7ed59).
6. One `SheetLocale`: the separator and ±0,000 are set options of the drawing set (0c4956b3,
   7fffa72c); date and sign are not moved yet.
7. One encoding decision and an explicit binding report: done (6e8937c5, c81539db,
   b5c0342b).

The function and module sizes of §5 are not reduced beyond what the fixes touched
(`compile_node` 571 → 568, `read_product` 1 385 → 1 384, `lib.rs` 2 358 → 2 357 lines).

## Tests on this branch

- Windows, full workspace `cargo test`: every suite passed except one plugin bridge
  test that failed under load (`run_plugin::a_link_in_or_as_a_plugin_folder…`). It
  passed when run again alone.
- The `run_plugin` module, run alone, times out five tests on this branch and on its base
  alike. It passes with `--test-threads=1`, so the cause is that those tests run at once,
  not these changes.
- Clippy with and without features, rustfmt and every `scripts/check_*.py` ratchet are clean.
- The Python tests: 393 passed, 48 skipped.
