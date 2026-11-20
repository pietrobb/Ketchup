import importlib.util
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_crate_layers", ROOT / "scripts" / "check_crate_layers.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def manifest(root, name, dependencies=(), dev=()):
    path = root / "crates" / name / "Cargo.toml"
    path.parent.mkdir(parents=True)
    lines = [f'[package]\nname = "{name}"\n\n[dependencies]\nserde = "1.0"']
    lines += [f'{d} = {{ path = "../{d}" }}' for d in dependencies]
    lines += ["\n[dev-dependencies]"] + [f'{d} = {{ path = "../{d}" }}' for d in dev]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def test_reads_only_normal_workspace_dependencies(tmp_path):
    manifest(tmp_path, "low")
    manifest(tmp_path, "high", ["low"], dev=["top"])
    assert checker.workspace_dependencies(tmp_path) == {"high": ["low"], "low": []}


def test_a_vendored_path_dependency_is_not_a_workspace_crate(tmp_path):
    manifest(tmp_path, "low")
    path = tmp_path / "crates" / "high" / "Cargo.toml"
    path.parent.mkdir(parents=True)
    path.write_text(
        '[package]\nname = "high"\n\n[dependencies]\nlow = { path = "../low" }\n'
        'vendored = { path = "../../third_party/vendored" }\n',
        encoding="utf-8",
    )
    assert checker.workspace_dependencies(tmp_path) == {"high": ["low"], "low": []}


def test_sideways_upward_and_unplaced_dependencies_fail():
    layers = {"low": 0, "mid": 1, "peer": 1}
    assert checker.violations({"mid": ["low"], "low": []}, layers) == []
    assert checker.violations({"mid": ["peer"]}, layers) == [
        "mid (layer 1) -> peer (layer 1): dependencies must point to a lower layer"
    ]
    assert checker.violations({"low": ["mid"]}, layers)[0].startswith("low (layer 0) -> mid")
    assert checker.violations({"new": []}, layers) == ["new: not placed in a layer"]


def test_workspace_follows_the_layers():
    graph = checker.workspace_dependencies(ROOT)
    assert checker.violations(graph) == []
    assert set(graph) == set(checker.LAYERS)


def test_assistant_pdm_and_analysis_are_never_below_the_model():
    layers = checker.LAYERS
    graph = checker.workspace_dependencies(ROOT)
    for crate in ["ketchup-assistant", "ketchup-pdm", "ketchup-analysis"]:
        assert layers[crate] > layers["ketchup-model"]
        assert crate not in graph["ketchup-model"]


def test_module_over_the_limit_fails_and_oversized_ones_only_shrink():
    oversized = {"big.rs": 30}
    assert checker.oversized_modules({"a.rs": 10, "big.rs": 30}, 10, oversized) == []
    assert checker.oversized_modules({"a.rs": 11, "big.rs": 30}, 10, oversized) == [
        "a.rs: 11 lines, limit 10; split it into modules"
    ]
    assert checker.oversized_modules({"big.rs": 31}, 10, oversized) == [
        "big.rs: 31 lines, limit 30; split it into modules"
    ]
    assert checker.oversized_modules({"big.rs": 20}, 10, oversized) == [
        "big.rs: shrank to 20 lines; lower OVERSIZED to it"
    ]
    assert checker.oversized_modules({"big.rs": 10}, 10, oversized) == [
        "big.rs: fits 10 lines now; remove it from OVERSIZED"
    ]


def test_a_production_module_past_the_freeze_size_fails_until_frozen():
    # Review 2026-11-08 TST-253: modules were frozen only after a review found them past
    # 3 000 lines; crossing it fails now, the module named. Test files are not frozen.
    sizes = {
        "crates/a/src/big.rs": 31,
        "crates/a/src/listed.rs": 40,
        "crates/a/src/small.rs": 30,
        "crates/a/src/big_tests.rs": 50,
        "crates/a/tests/flow.rs": 50,
    }
    assert checker.unfrozen_large_modules(sizes, 30, {"crates/a/src/listed.rs": 40}) == [
        "crates/a/src/big.rs: 31 lines, over 30; split it or add it to FROZEN_MODULES"
    ]


def test_the_frozen_list_may_only_go_down_from_the_commit_before(tmp_path):
    # Review 2026-11-09 R1109-TST-305: each change wrote its own module's new size into
    # FROZEN_MODULES (`live_bridge.rs` 3550 to 3635 in the change that grew it).
    base = {"crates/a/src/frozen.rs": 3550, "crates/a/src/kept.rs": 3200}
    base_sizes = {"crates/a/src/crossed.rs": 2900, "crates/a/src/grown.rs": 2990}
    frozen = {
        "crates/a/src/frozen.rs": 3635,
        "crates/a/src/kept.rs": 3100,
        "crates/a/src/crossed.rs": 3000,
        "crates/a/src/grown.rs": 3200,
    }
    assert checker.raised_frozen_entries(frozen, base, base_sizes, 3000) == [
        "crates/a/src/frozen.rs: frozen at 3550 before this change, 3635 now; "
        "FROZEN_MODULES may only go down, put new code in a new module",
        "crates/a/src/grown.rs: added to FROZEN_MODULES at 3200, past 3000; "
        "split it so it stays under 3000 lines",
    ]
    text = 'FREEZE = 1\nFROZEN_MODULES = {\n    # A comment.\n    "a.rs": 3,\n}\n'
    assert checker.frozen_modules_of(text) == {"a.rs": 3}


def git(root, *arguments):
    return subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
         *arguments],
        cwd=root, check=True, capture_output=True, text=True,
    ).stdout.strip()


def commit_frozen(root, frozen, message):
    script = root / "scripts" / "check_crate_layers.py"
    script.parent.mkdir(exist_ok=True)
    script.write_text(f"FROZEN_MODULES = {frozen!r}\n", encoding="utf-8")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", message)
    return git(root, "rev-parse", "HEAD")


def test_an_entry_raised_in_an_earlier_commit_of_the_branch_still_fails(tmp_path):
    # Review 2026-11-10 R1110-TST-400: compared with HEAD^1 only, commit A raising
    # `viewport.rs` from 4857 to 4950 failed, and passed once commit B followed it or
    # main was merged into the branch.
    git(tmp_path, "init", "-q")
    main = commit_frozen(tmp_path, {"viewport.rs": 4857}, "main")
    git(tmp_path, "update-ref", "refs/remotes/origin/main", main)
    commit_frozen(tmp_path, {"viewport.rs": 4950}, "A")
    (tmp_path / "other.txt").write_text("B\n", encoding="utf-8")
    git(tmp_path, "add", "-A")
    git(tmp_path, "commit", "-q", "-m", "B")
    raised = ["viewport.rs: frozen at 4857 before this change, 4950 now; "
              "FROZEN_MODULES may only go down, put new code in a new module"]
    frozen = {"viewport.rs": 4950}
    assert checker.frozen_list_problems(tmp_path, frozen=frozen, environ={}) == raised
    # A push to main is compared with main before it, which CI names.
    git(tmp_path, "update-ref", "refs/remotes/origin/main", "HEAD")
    assert checker.frozen_list_problems(tmp_path, frozen=frozen, environ={}) == []
    pushed = {"KETCHUP_FREEZE_BASE": main}
    assert checker.frozen_list_problems(tmp_path, frozen=frozen, environ=pushed) == raised


def test_without_origin_main_only_a_merge_commit_is_compared_with_its_first_parent(tmp_path):
    # Review 2026-11-12 P3: without origin/main the check fell back to HEAD^1 silently,
    # so commit A raising an entry passed once commit B followed it.
    git(tmp_path, "init", "-q", "-b", "main")
    commit_frozen(tmp_path, {"viewport.rs": 4857}, "main")
    git(tmp_path, "checkout", "-q", "-b", "branch")
    commit_frozen(tmp_path, {"viewport.rs": 4950}, "A")
    (tmp_path / "other.txt").write_text("B\n", encoding="utf-8")
    git(tmp_path, "add", "-A")
    git(tmp_path, "commit", "-q", "-m", "B")
    frozen = {"viewport.rs": 4950}
    problems = checker.frozen_list_problems(tmp_path, frozen=frozen, environ={})
    assert len(problems) == 1 and "KETCHUP_FREEZE_BASE" in problems[0], problems
    # A pull request's merge commit has the target branch as its first parent.
    git(tmp_path, "checkout", "-q", "main")
    git(tmp_path, "-c", "core.editor=true", "merge", "-q", "--no-ff", "branch")
    assert checker.frozen_list_problems(tmp_path, frozen=frozen, environ={}) == [
        "viewport.rs: frozen at 4857 before this change, 4950 now; "
        "FROZEN_MODULES may only go down, put new code in a new module"]


def test_workspace_frozen_list_did_not_grow_since_the_commit_before():
    assert checker.frozen_list_problems(ROOT) == []


def test_workspace_modules_past_the_freeze_size_are_frozen():
    assert checker.unfrozen_large_modules(checker.module_sizes(ROOT)) == []


def test_workspace_modules_fit_the_limit():
    sizes = checker.module_sizes(ROOT)
    assert checker.oversized_modules(sizes) == []
    for module in [
        "crates/ketchup-model/src/document.rs",
        "crates/ketchup-model/src/exact_brep_graph.rs",
        "crates/ketchup-geometry/src/sketch.rs",
    ]:
        assert sizes[module] <= checker.MAX_MODULE_LINES
        assert module not in checker.OVERSIZED


def test_integration_test_files_are_held_to_the_module_limit():
    # Review TST-1: a test file of 15 000 lines grew because only src was capped.
    sizes = checker.module_sizes(ROOT)
    assert "crates/ketchup-app/tests/headless_shell.rs" in sizes
    assert "crates/ketchup-app/tests/scale_grips_ui.rs" in sizes
    assert all(
        sizes[path] <= checker.MAX_MODULE_LINES or path in checker.OVERSIZED
        for path in sizes
        if "/tests/" in path
    )


def test_function_lengths_skip_inline_test_modules_and_bodiless_functions(tmp_path):
    module = tmp_path / "m.rs"
    module.write_text(
        "pub(crate) fn short(\n    a: u8,\n) -> u8 {\n    a\n}\n"
        "trait T {\n    fn declared(&self);\n}\n"
        "impl T for u8 {\n    fn declared(&self) {\n        let _ = 1;\n    }\n}\n"
        "#[cfg(test)]\nmod tests {\n    fn very_long_test() {\n\n\n\n    }\n}\n",
        encoding="utf-8",
    )
    assert checker.function_lengths(module) == {"short": 5, "declared": 3}


def test_function_over_the_limit_fails_and_long_ones_only_shrink():
    recorded = {"m.rs::long": 30}
    assert checker.oversized_functions({"m.rs::long": 30}, recorded, 10) == []
    assert checker.oversized_functions({"m.rs::new": 11, "m.rs::long": 30}, recorded, 10) == [
        "m.rs::new: 11 lines, limit 10; split it into named steps"
    ]
    assert checker.oversized_functions({"m.rs::long": 31}, recorded, 10) == [
        "m.rs::long: 31 lines, limit 30; split it into named steps"
    ]
    assert checker.oversized_functions({"m.rs::long": 20}, recorded, 10) == [
        "m.rs::long: shrank to 20 lines; lower LONG_FUNCTIONS to it"
    ]
    assert checker.oversized_functions({}, recorded, 10) == [
        "m.rs::long: fits 10 lines now; remove it from LONG_FUNCTIONS"
    ]


def test_test_modules_are_told_from_production_modules():
    for path in ["crates/a/tests/x.rs", "crates/a/src/tests.rs", "crates/a/src/tests/x.rs",
                 "crates/a/src/live_bridge/product_integration_tests.rs"]:
        assert checker.is_test_module(Path(path))
    for path in ["crates/a/src/lib.rs", "crates/a/src/testing.rs", "crates/a/src/test_env.rs"]:
        assert not checker.is_test_module(Path(path))


def test_test_over_the_limit_fails_and_long_ones_only_shrink():
    recorded = {"t.rs::long": 30}
    assert checker.oversized_tests({"t.rs::long": 30}, recorded, 10) == []
    assert checker.oversized_tests({"t.rs::new": 11}, recorded, 10)[0] == (
        "t.rs::new: 11 lines, limit 10; split it into one test per behavior"
    )
    assert checker.oversized_tests({"t.rs::long": 20}, recorded, 10) == [
        "t.rs::long: shrank to 20 lines; lower LONG_TESTS to it"
    ]


def test_workspace_tests_fit_the_limit_or_only_shrink():
    lengths = checker.long_tests(ROOT)
    assert checker.oversized_tests(lengths) == []
    assert not [name for name in lengths if name.startswith("crates/ketchup-app/src/tests.rs")]
    assert "crates/ketchup-app/src/tests.rs" not in checker.OVERSIZED


def test_app_shell_has_no_oversized_module_or_function():
    lengths = checker.long_functions(ROOT)
    assert checker.oversized_functions(lengths) == []
    assert not [name for name in lengths if name.startswith("crates/ketchup-app/src/app")]
    assert "crates/ketchup-app/src/lib.rs" not in checker.OVERSIZED
    assert checker.module_sizes(ROOT)["crates/ketchup-app/src/lib.rs"] <= checker.MAX_MODULE_LINES


def test_geometry_depends_on_nothing_but_the_tolerances():
    graph = checker.workspace_dependencies(ROOT)
    assert graph["ketchup-geometry"] == ["ketchup-tolerance"]
    assert checker.LAYERS["ketchup-geometry"] < checker.LAYERS["ketchup-model"]


def test_milestone_named_test_files_fail_and_behavior_names_pass():
    named = [
        "crates/a/tests/gate_d.rs",
        "crates/a/tests/gate_c1a_projection_authority.rs",
        "crates/a/tests/assistant_m7.rs",
        "crates/a/tests/m120_cut.rs",
        "crates/a/tests/live_bridge_s4.rs",
        "crates/a/src/plugin_m7b_tests.rs",
    ]
    behavior = [
        "crates/a/tests/push_pull.rs",
        "crates/a/tests/save_reopen.rs",
        "crates/a/tests/three_mf_export.rs",
        "crates/a/tests/gateway_routing.rs",
        "crates/a/src/assistant_deadline_tests.rs",
    ]
    problems = checker.milestone_named_tests(named + behavior)
    assert [problem.split(":")[0] for problem in problems] == named


def test_workspace_test_files_are_named_after_behavior():
    files = checker.test_files(ROOT)
    assert "crates/ketchup-scheduler/tests/scheduler_reliability.rs" in files
    assert checker.milestone_named_tests(files) == []


def test_hand_written_linear_algebra_is_counted_and_only_falls(tmp_path):
    source = tmp_path / "crates" / "a" / "src" / "lib.rs"
    source.parent.mkdir(parents=True)
    source.write_text(
        "fn cross3(a: V, b: V) {}\n"
        "let n = [u[1] * v[2] - u[2] * v[1], 0.0, 0.0];\n"
        "let p = m[0] * q[0] + m[1] * q[1] + m[3];\n"
        "let determinant = a * d - b * c;\n"
        "let determinant = linear.determinant();\n"
        "let product = dot(a, cross(b, c));\n"
        "fn normalize(vector: [f32; 3]) -> [f32; 3] {}\n"
        "fn sub(a: Point, b: Point) -> Point {}\n"
        "pub fn transform_point(self, point: [f64; 3]) -> [f64; 3] {}\n"
        "fn normalize(shape: &mut Shape) {}\n",
        encoding="utf-8",
    )
    shared = tmp_path / "crates" / "ketchup-geometry" / "src" / "linalg.rs"
    shared.parent.mkdir(parents=True)
    shared.write_text("fn dot(a: V, b: V) {}\n", encoding="utf-8")
    counts = checker.linear_algebra_counts(tmp_path)
    assert counts == {"crates/a/src/lib.rs": 6}
    assert checker.hand_written_linear_algebra(counts, {"crates/a/src/lib.rs": 6}) == []
    assert len(checker.hand_written_linear_algebra(counts, {})) == 1
    assert "lower LINEAR_ALGEBRA" in checker.hand_written_linear_algebra(
        counts, {"crates/a/src/lib.rs": 7}
    )[0]


def test_numeric_casts_and_lint_allows_are_counted_per_crate_and_only_fall(tmp_path):
    source = tmp_path / "crates" / "a" / "src" / "lib.rs"
    source.parent.mkdir(parents=True)
    source.write_text(
        "#[allow(clippy::too_many_arguments)]\n"
        "fn f(x: f64) -> u32 { (x as u32) + (x as usize as u32) } // y as f32\n"
        "use std::fmt as format; let r = &x as &dyn Any;\n"
        "#[cfg(test)]\nmod tests {\n    #[allow(dead_code)]\n    const N: u8 = 1.0 as u8;\n}\n",
        encoding="utf-8",
    )
    (tmp_path / "crates" / "a" / "src" / "lib_tests.rs").write_text("let n = 1.0 as u8;\n", encoding="utf-8")
    counts = checker.cast_and_allow_counts(tmp_path)
    assert counts == {"a casts": 3, "a allows": 1}
    assert checker.grown_casts_and_allows(counts, {"a casts": 3, "a allows": 1}) == []
    assert "limit 2" in checker.grown_casts_and_allows(counts, {"a casts": 2, "a allows": 1})[0]
    assert "lower CASTS_AND_ALLOWS" in checker.grown_casts_and_allows(counts, {"a casts": 4, "a allows": 1})[0]


def test_unsafe_under_an_allow_already_counted_is_counted_itself(tmp_path):
    # Review 2026-11-13 P3: four unsafe blocks under one module-wide allow left the
    # counts as they were.
    source = tmp_path / "crates" / "a" / "src" / "lib.rs"
    source.parent.mkdir(parents=True)
    source.write_text(
        "#![allow(unsafe_code)]\n"
        "fn f() { unsafe { g() }; let x = unsafe { h() }; } // unsafe { i() }\n"
        "unsafe fn g() {}\nstruct S; unsafe impl Send for S {}\n"
        'fn m() -> &\'static str { "a path is unsafe \\" unsafe" }\n'
        "#[cfg(test)]\nmod tests {\n    fn t() { unsafe { j() } }\n}\n",
        encoding="utf-8",
    )
    counts = checker.cast_and_allow_counts(tmp_path)
    assert counts == {"a allows": 1, "a unsafe": 4}
    assert "limit 3" in checker.grown_casts_and_allows(counts, {"a allows": 1, "a unsafe": 3})[0]


def test_unsafe_in_a_vendored_crate_is_counted_and_said_as_unsafe(tmp_path):
    # Review 2026-11-13 r95 P3: the patched starlark and process-wrap went uncounted, and a
    # grown count of unsafe code was answered with advice about casts.
    vendored = tmp_path / "third_party" / "w" / "src"
    vendored.mkdir(parents=True)
    (vendored / "lib.rs").write_text(
        "unsafe fn f() {}\nfn g() { unsafe { f() } }\n"
        "#[cfg(test)]\nmod tests {\n    fn t() { unsafe { super::f() } }\n}\n",
        encoding="utf-8",
    )
    (tmp_path / "third_party" / "w" / "tests").mkdir()
    (tmp_path / "third_party" / "w" / "tests" / "t.rs").write_text("unsafe fn h() {}\n", encoding="utf-8")
    counts = checker.cast_and_allow_counts(tmp_path)
    assert counts == {"third_party/w unsafe": 2}
    [grown] = checker.grown_casts_and_allows(counts, {"third_party/w unsafe": 1})
    assert "limit 1" in grown and "safe Rust" in grown and "TryFrom" not in grown


def test_workspace_casts_and_allows_only_fall():
    assert checker.grown_casts_and_allows(checker.cast_and_allow_counts(ROOT)) == []


def test_a_test_switch_in_a_release_binary_fails_until_listed_with_its_reason(tmp_path):
    """Review 2026-10-29 SEC-62, CI-63: no check saw a test switch in the release binary."""
    source = tmp_path / "crates" / "a" / "src"
    source.mkdir(parents=True)
    (source / "lib.rs").write_text(
        'const ADAPTER: &str = "KETCHUP_TEST_ADAPTER";\n'
        '#[cfg(test)]\nmod tests {\n    const DIR: &str = "KETCHUP_TEST_DIR";\n}\n',
        encoding="utf-8",
    )
    (source / "native.cc").write_text('std::getenv("KETCHUP_TEST_NATIVE");\n', encoding="utf-8")
    (source / "lib_tests.rs").write_text('const T: &str = "KETCHUP_TEST_T";\n', encoding="utf-8")
    helper = tmp_path / "crates" / "ketchup-test-env" / "src"
    helper.mkdir(parents=True)
    (helper / "lib.rs").write_text('const R: &str = "KETCHUP_TEST_REPORT";\n', encoding="utf-8")
    found = checker.release_test_switches(tmp_path)
    assert found == {
        "KETCHUP_TEST_ADAPTER": "crates/a/src/lib.rs",
        "KETCHUP_TEST_NATIVE": "crates/a/src/native.cc",
    }
    problems = checker.unlisted_test_switches(found, {"KETCHUP_TEST_ADAPTER": "harmless"})
    assert problems == [
        "crates/a/src/native.cc: KETCHUP_TEST_NATIVE is a test switch in a release binary; "
        "keep it in test code, or list it in TEST_SWITCHES with why it is harmless"
    ]
    listed = dict.fromkeys(["KETCHUP_TEST_ADAPTER", "KETCHUP_TEST_NATIVE", "KETCHUP_TEST_OLD"], "")
    assert checker.unlisted_test_switches(found, listed) == [
        "KETCHUP_TEST_OLD is no longer read; remove it from TEST_SWITCHES"
    ]


def test_workspace_test_switches_in_release_binaries_are_listed():
    assert checker.unlisted_test_switches(checker.release_test_switches(ROOT)) == []


def test_workspace_linear_algebra_stays_in_ketchup_geometry():
    counts = checker.linear_algebra_counts(ROOT)
    assert checker.hand_written_linear_algebra(counts) == []


def test_a_program_body_without_geometry_fails():
    source = (
        "pub enum ProgramPartBody {\n    Panel,\n    Extrusion {\n        segments: Vec<S>,\n"
        "    },\n    Sphere,\n}\n"
    )
    problems = checker.named_program_bodies(source)
    assert [problem.split()[0] for problem in problems] == [
        "ProgramPartBody::Panel",
        "ProgramPartBody::Sphere",
    ]


def test_workspace_program_bodies_carry_their_geometry():
    source = (ROOT / "crates/ketchup-program/src/model.rs").read_text(encoding="utf-8")
    assert checker.named_program_bodies(source) == []


def test_an_operation_kind_kept_beside_the_operations_fails():
    source = (
        "pub enum ProgramOperation {\n    Cut(ProgramCut),\n    Boolean(Box<ProgramBoolean>),\n"
        "    /// Drilled.\n    Hole(Hole),\n}\n"
        "pub struct Part {\n    pub operations: Vec<ProgramOperation>,\n"
        "    pub features: Vec<ProgramFeature>,\n    pub holes: Vec<Hole>,\n"
        "    pub tools: Vec<ProgramBoolean>,\n}\n"
    )
    problems = checker.operations_beside_operations(source)
    assert [problem.split(":")[0] for problem in problems] == ["Part.holes", "Part.tools"]


def test_workspace_part_keeps_every_operation_in_program_order():
    source = (ROOT / "crates/ketchup-program/src/model.rs").read_text(encoding="utf-8")
    assert checker.operations_beside_operations(source) == []
