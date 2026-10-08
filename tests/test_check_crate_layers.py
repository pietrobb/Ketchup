import importlib.util
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


def test_workspace_casts_and_allows_only_fall():
    assert checker.grown_casts_and_allows(checker.cast_and_allow_counts(ROOT)) == []


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
