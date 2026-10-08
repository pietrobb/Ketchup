"""Workspace crates form layers: a crate may depend only on crates of a lower layer.

The table below is the one statement of the architecture. A new crate must be
placed in it, and a dependency that points sideways or upward fails, so the
assistant and PDM crates can never become dependencies of the model or geometry.
Only normal `[dependencies]` count; tests may use any crate.

No source module of any crate may exceed MAX_MODULE_LINES. Modules that were
already larger are listed in OVERSIZED with their size: they may shrink but not
grow, and an entry must be removed once its module fits the limit. Production
functions follow the same rule with MAX_FUNCTION_LINES and LONG_FUNCTIONS: a
long function is split into named steps, never extended.

Test files are named after the behavior they check (push_pull, save_reopen),
never after a milestone, gate or slice (gate_d, m120_, assistant_m7, live_bridge_s4).
"""

import re
import sys
import tomllib
from pathlib import Path

LAYERS = {
    "ketchup-tolerance": 0,
    "ketchup-rejection": 0,
    "ketchup-test-env": 0,
    "ketchup-geometry": 1,
    "ketchup-model": 2,
    "ketchup-exact": 2,
    "ketchup-interaction": 3,
    "ketchup-assistant": 3,
    "ketchup-analysis": 3,
    "ketchup-pdm": 3,
    "ketchup-manufacturing": 3,
    "ketchup-program": 4,
    "ketchup-scheduler": 4,
    "ketchup-application": 5,
    "ketchup-mcp": 5,
    "ketchup-headless": 6,
    "ketchup-app": 6,
}

MAX_MODULE_LINES = 5_000

OVERSIZED = {}

MAX_FUNCTION_LINES = 400

# Production functions that were already longer, with their length. They may
# shrink but not grow; an entry is removed once its function fits the limit.
LONG_FUNCTIONS = {
    "crates/ketchup-app/src/assembly_ui.rs::show_assembly_editor_content": 756,
    "crates/ketchup-app/src/feature_history_ui.rs::show_feature_history_content": 672,
    "crates/ketchup-application/src/append_feature.rs::plan_feature_kind": 793,
    "crates/ketchup-application/src/collision.rs::collision_report": 719,
    "crates/ketchup-application/src/creation.rs::plan_creation": 411,
    "crates/ketchup-application/src/planner.rs::plan_assistant_cad_edit_program_with_outputs": 1988,
    "crates/ketchup-application/src/validation.rs::assistant_assembly_retention_report": 455,
    "crates/ketchup-application/src/validation.rs::assistant_hardware_manufacturing_report": 431,
    "crates/ketchup-application/src/validation.rs::assistant_validation_context_base": 634,
    "crates/ketchup-assistant/src/intent.rs::propose_intent": 968,
    "crates/ketchup-model/src/document/digest_v3.rs::feature_kind": 702,
    "crates/ketchup-model/src/document/feature_validation.rs::validate_feature_kind": 408,
    "crates/ketchup-model/src/document/product_validation.rs::validate_features": 708,
    "crates/ketchup-model/src/document/proposal_analysis.rs::authoritative_dependencies": 904,
    "crates/ketchup-model/src/document/solid_tool.rs::clone_definition_and_repoint": 477,
    "crates/ketchup-model/src/document/store.rs::apply_batch_with_origin_and_validation": 2079,
    "crates/ketchup-model/src/exact_brep_graph/compiler.rs::compile_node": 615,
    "crates/ketchup-model/src/persistence/legacy.rs::read_product": 1396,
    "crates/ketchup-model/src/shared_change.rs::commit_occurrence_fork_change": 558,
    "crates/ketchup-model/src/shared_change.rs::project_occurrence_fork_impact": 457,
    "crates/ketchup-program/src/eval.rs::builtins": 833,
    "crates/ketchup-scheduler/src/exact_worker.rs::evaluate_exact_brep_graph": 559,
}

# A test checks one behavior; a longer one is several scenarios in one, and its
# first failing assert hides whether the rest still work.
MAX_TEST_LINES = 300

# Tests that were already longer, with their length. They may shrink but not
# grow; an entry is removed once its test fits the limit.
LONG_TESTS = {
    "crates/ketchup-app/src/live_bridge/product_integration_tests.rs::original_v9_nightstand_guarded_physical_repair_has_one_undo_and_verified_geometry": 489,
    "crates/ketchup-app/src/tests/exact_modeling.rs::contained_circle_subtract_intersect_split_and_containing_union_round_trip_atomically": 335,
    "crates/ketchup-app/src/tests/exact_modeling.rs::contained_slanted_polygon_solid_tools_round_trip_atomically": 398,
    "crates/ketchup-app/src/tests/exact_modeling.rs::mixed_extrusion_and_imported_exact_occurrences_route_through_solid_tools": 321,
    "crates/ketchup-app/tests/assistant_workflows.rs::integrated_finishing_chain_rebuilds_exactly_through_headless_assistant": 333,
    "crates/ketchup-app/tests/capstone_chain.rs::empty_document_manual_ux_capstone_has_rendered_and_native_exact_evidence": 305,
    "crates/ketchup-app/tests/capstone_chain.rs::the_manual_capstone_runs_end_to_end_through_the_designed_shell": 346,
    "crates/ketchup-app/tests/face_workflow_ui.rs::line_click_preview_exact_length_cancel_undo_and_save_open_are_canonical": 419,
    "crates/ketchup-app/tests/file_workflow.rs::file_import_dxf_reviews_and_commits_one_canonical_profile_transaction_offscreen": 471,
    "crates/ketchup-app/tests/file_workflow.rs::file_import_exact_step_preserves_a_real_nested_repeated_xde_assembly_offscreen": 421,
    "crates/ketchup-app/tests/instanced_rendering.rs::garden_studio_hardware_gpu_camera_frames": 345,
    "crates/ketchup-app/tests/timber_frame_house.rs::live_oauth_assistant_builds_a_roofed_house_frame_across_turns": 714,
    "crates/ketchup-application/tests/cad_program.rs::one_physical_pin_joint_operation_creates_both_hole_rows_atomically": 305,
    "crates/ketchup-application/tests/cad_program.rs::public_nested_assembly_joint_motion_drawing_round_trip_is_branch_exact": 371,
    "crates/ketchup-application/tests/evaluation_deadline.rs::physical_recipe_save_open_history_recomputes_full_exact_without_cached_evidence": 396,
    "crates/ketchup-application/tests/workflow_trace.rs::original_shared_side_make_unique_preserves_physical_holes_and_recipe": 332,
    "crates/ketchup-model/tests/pad_pocket.rs::face_supported_pocket_and_topology_history_make_unique_losslessly": 453,
    "crates/ketchup-model/tests/workplane_sketch.rs::all_principal_planes_and_one_resolved_planar_face_support_are_canonical": 310,
    "crates/ketchup-scheduler/tests/exact_brep_graph.rs::through_cut_uses_safe_bounds_for_revolve_loft_and_imported_exact_bodies": 336,
    "crates/ketchup-scheduler/tests/exact_brep_graph.rs::worker_binds_multiple_imported_sources_by_digest_for_boolean_and_mesh": 375,
    "crates/ketchup-scheduler/tests/exact_brep_graph.rs::worker_rebinds_topology_selected_finishes_and_rejects_lost_provenance": 376,
}


# A milestone, gate or slice label as one word of a snake_case file name.
MILESTONE_WORD = re.compile(r"(?:^|_)(?:gate(?:_[a-z]?\d+[a-z]?|_[a-z])?|[ms]\d+[a-z]?)(?:_|$)")


def workspace_dependencies(root):
    """{crate: [workspace crates it depends on]} from every crates/*/Cargo.toml."""
    graph = {}
    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        data = tomllib.loads(manifest.read_text(encoding="utf-8"))
        dependencies = data.get("dependencies", {})
        graph[data["package"]["name"]] = sorted(
            name
            for name, spec in dependencies.items()
            if isinstance(spec, dict) and "path" in spec
        )
    return graph


def violations(graph, layers=LAYERS):
    problems = []
    for crate, dependencies in graph.items():
        if crate not in layers:
            problems.append(f"{crate}: not placed in a layer")
            continue
        for dependency in dependencies:
            if dependency not in layers:
                problems.append(f"{crate} -> {dependency}: {dependency} not placed in a layer")
            elif layers[dependency] >= layers[crate]:
                problems.append(
                    f"{crate} (layer {layers[crate]}) -> {dependency} "
                    f"(layer {layers[dependency]}): dependencies must point to a lower layer"
                )
    return problems


def module_sizes(root):
    """{"crates/<crate>/src/...rs": line count} for every crate source module."""
    return {
        path.relative_to(root).as_posix(): len(path.read_text(encoding="utf-8").splitlines())
        for path in sorted((root / "crates").glob("*/src/**/*.rs"))
    }


def oversized_modules(sizes, limit=MAX_MODULE_LINES, oversized=OVERSIZED):
    problems = []
    for module, lines in sizes.items():
        allowed = oversized.get(module, limit)
        if lines > allowed:
            problems.append(f"{module}: {lines} lines, limit {allowed}; split it into modules")
    for module, recorded in oversized.items():
        if sizes.get(module, 0) <= limit:
            problems.append(f"{module}: fits {limit} lines now; remove it from OVERSIZED")
        elif sizes[module] < recorded:
            problems.append(f"{module}: shrank to {sizes[module]} lines; lower OVERSIZED to it")
    return problems


# A function item, rustfmt-formatted: its body closes on a `}` at the same indent.
FUNCTION = re.compile(r'^(\s*)(?:pub(?:\([^)]*\))? )?(?:(?:const|async|unsafe|extern "C")\s+)*fn (\w+)')
TEST_MODULE = re.compile(r"^(\s*)mod \w+ \{$")


def function_lengths(path):
    """{name: lines} for every function of one module, inline test modules skipped."""
    lines = path.read_text(encoding="utf-8").splitlines()
    lengths = {}
    index = 0
    while index < len(lines):
        test_module = TEST_MODULE.match(lines[index])
        if test_module and index and lines[index - 1].strip() == "#[cfg(test)]":
            index = lines.index(test_module.group(1) + "}", index + 1) + 1
            continue
        function = FUNCTION.match(lines[index])
        if function:
            indent, name = function.groups()
            opening = index
            while not lines[opening].rstrip().endswith(("{", ";", "}")):
                opening += 1
            if lines[opening].rstrip().endswith("{"):
                closing = lines.index(indent + "}", opening + 1)
                lengths[name] = max(lengths.get(name, 0), closing - index + 1)
        index += 1
    return lengths


def is_test_module(relative):
    """A file of tests: under a `tests` directory, `tests.rs` or `*_tests.rs`."""
    return "tests" in relative.parts[:-1] or relative.stem == "tests" or relative.stem.endswith("_tests")


def long_functions(root, limit=MAX_FUNCTION_LINES):
    """{"crates/<crate>/src/...rs::name": lines} for production functions over the limit."""
    return {
        f"{path.relative_to(root).as_posix()}::{name}": length
        for path in sorted((root / "crates").glob("*/src/**/*.rs"))
        if not is_test_module(path.relative_to(root))
        for name, length in function_lengths(path).items()
        if length > limit
    }


def long_tests(root, limit=MAX_TEST_LINES):
    """{"crates/<crate>/...rs::name": lines} for functions of test files over the limit."""
    return {
        f"{path.relative_to(root).as_posix()}::{name}": length
        for path in sorted((root / "crates").glob("*/**/*.rs"))
        if "target" not in path.relative_to(root).parts and is_test_module(path.relative_to(root))
        for name, length in function_lengths(path).items()
        if length > limit
    }


def oversized_functions(
    lengths,
    recorded=LONG_FUNCTIONS,
    limit=MAX_FUNCTION_LINES,
    table="LONG_FUNCTIONS",
    remedy="split it into named steps",
):
    problems = []
    for function, length in lengths.items():
        allowed = recorded.get(function, limit)
        if length > allowed:
            problems.append(f"{function}: {length} lines, limit {allowed}; {remedy}")
    for function, length in recorded.items():
        if function not in lengths:
            problems.append(f"{function}: fits {limit} lines now; remove it from {table}")
        elif lengths[function] < length:
            problems.append(f"{function}: shrank to {lengths[function]} lines; lower {table} to it")
    return problems


def oversized_tests(lengths, recorded=LONG_TESTS, limit=MAX_TEST_LINES):
    return oversized_functions(
        lengths, recorded, limit, "LONG_TESTS", "split it into one test per behavior"
    )


def test_files(root):
    """Every Rust test file: integration tests and unit-test modules of each crate."""
    return [
        path.relative_to(root).as_posix()
        for pattern in ("*/tests/**/*.rs", "*/src/**/*tests*.rs")
        for path in sorted((root / "crates").glob(pattern))
    ]


def milestone_named_tests(paths):
    return [
        f"{path}: name the test file after the behavior it checks, not a milestone or gate"
        for path in paths
        if MILESTONE_WORD.search(Path(path).stem)
    ]


# Vector and matrix arithmetic lives in ketchup_geometry::linalg; written out
# again elsewhere it drifts (singularity thresholds, row/column layout).
HAND_WRITTEN_LINEAR_ALGEBRA = (
    re.compile(r"\bfn (?:dot|cross|determinant)\d*\("),
    re.compile(r"\b(\w+)\[1\] \* (\w+)\[2\] - \1\[2\] \* \2\[1\]"),
    re.compile(r"\b(\w+)\[0\] \* (\w+)\[0\] \+ \1\[1\] \* \2\[1\]"),
    re.compile(r"\blet determinant = (?!.*\b(?:determinant|dot|cross2?)\()"),
    # A free vector helper (methods of a type such as Transform are its API).
    re.compile(
        r"\bfn (?:normalize|sub|transform_point)\d*\((?!&?self\b)\w+: (?:\[f(?:32|64); [23]\]|Point\b|Vec3\b)"
    ),
)
# The remaining hand-written spots per file; the counts may only fall.
LINEAR_ALGEBRA = {
    "crates/ketchup-analysis/src/fea.rs": 1,
    # GPU single-precision wrapper over linalg::normalize.
    "crates/ketchup-app/src/renderer.rs": 1,
    # 2D wrapper over Affine3::transform_point that checks the coordinate range.
    "crates/ketchup-manufacturing/src/dxf_export.rs": 1,
    # 2D point difference; linalg has dot2/cross2 but no 2D subtraction.
    "crates/ketchup-program/src/contact_polygon.rs": 1,
}


def linear_algebra_counts(root):
    """{"crates/...rs": hits} for hand-written linear algebra outside ketchup-geometry."""
    counts = {}
    for path in sorted((root / "crates").glob("*/**/*.rs")):
        module = path.relative_to(root).as_posix()
        if module.startswith("crates/ketchup-geometry/"):
            continue
        hits = sum(
            1
            for line in path.read_text(encoding="utf-8").splitlines()
            if any(pattern.search(line) for pattern in HAND_WRITTEN_LINEAR_ALGEBRA)
        )
        if hits:
            counts[module] = hits
    return counts


def hand_written_linear_algebra(counts, recorded=LINEAR_ALGEBRA):
    problems = []
    for module, hits in counts.items():
        allowed = recorded.get(module, 0)
        if hits > allowed:
            problems.append(
                f"{module}: {hits} hand-written dot/cross/determinant/matrix products, "
                f"limit {allowed}; use ketchup_geometry::linalg"
            )
    for module, allowed in recorded.items():
        if counts.get(module, 0) < allowed:
            problems.append(
                f"{module}: hand-written linear algebra fell to {counts.get(module, 0)}; "
                "lower LINEAR_ALGEBRA to it"
            )
    return problems


# Numeric `as` casts truncate, wrap or round without saying so, and `#[allow(` switches a
# lint off; both are counted per crate in production source and may only fall.
NUMERIC_CAST = re.compile(r"\bas\s+(?:[ui](?:8|16|32|64|128|size)|f32|f64)\b")
LINT_ALLOW = re.compile(r"#!?\[allow\(")
CASTS_AND_ALLOWS = {
    "ketchup-analysis allows": 1,
    "ketchup-analysis casts": 6,
    "ketchup-app allows": 10,
    "ketchup-app casts": 251,
    "ketchup-application allows": 9,
    "ketchup-application casts": 38,
    "ketchup-assistant allows": 3,
    "ketchup-assistant casts": 8,
    "ketchup-exact allows": 4,
    "ketchup-exact casts": 43,
    "ketchup-geometry allows": 1,
    "ketchup-geometry casts": 5,
    "ketchup-headless casts": 4,
    "ketchup-interaction casts": 21,
    "ketchup-manufacturing allows": 11,
    "ketchup-manufacturing casts": 43,
    "ketchup-mcp allows": 2,
    "ketchup-mcp casts": 10,
    "ketchup-model allows": 16,
    "ketchup-model casts": 419,
    "ketchup-pdm casts": 5,
    "ketchup-program allows": 10,
    "ketchup-program casts": 53,
    "ketchup-scheduler allows": 3,
    "ketchup-scheduler casts": 86,
}


def production_lines(path):
    """The lines of a production module with its inline `#[cfg(test)] mod` removed."""
    lines = path.read_text(encoding="utf-8").splitlines()
    kept = []
    index = 0
    while index < len(lines):
        test_module = TEST_MODULE.match(lines[index])
        if test_module and index and lines[index - 1].strip() == "#[cfg(test)]":
            index = lines.index(test_module.group(1) + "}", index + 1) + 1
            continue
        kept.append(lines[index])
        index += 1
    return kept


def cast_and_allow_counts(root):
    """{"<crate> casts"|"<crate> allows": count} over production source."""
    counts = {}
    for path in sorted((root / "crates").glob("*/src/**/*.rs")):
        relative = path.relative_to(root)
        if is_test_module(relative):
            continue
        crate = relative.parts[1]
        for line in production_lines(path):
            code = line.split("//", 1)[0]
            for kind, pattern in (("casts", NUMERIC_CAST), ("allows", LINT_ALLOW)):
                found = len(pattern.findall(code if kind == "casts" else line))
                if found:
                    counts[f"{crate} {kind}"] = counts.get(f"{crate} {kind}", 0) + found
    return counts


def grown_casts_and_allows(counts, recorded=CASTS_AND_ALLOWS):
    problems = []
    for key in sorted(set(counts) | set(recorded)):
        found, allowed = counts.get(key, 0), recorded.get(key, 0)
        if found > allowed:
            problems.append(
                f"{key}: {found}, limit {allowed}; use From/TryFrom or a named conversion, "
                "and fix the lint instead of allowing it"
            )
        elif found < allowed:
            problems.append(f"{key}: fell to {found}; lower CASTS_AND_ALLOWS to it")
    return problems


# A program body carries its own geometry (profile, path, sections); a
# variant without data is a shape fixed by name and size, as Panel was.
PROGRAM_BODY = re.compile(r"pub enum ProgramPartBody \{(.*?)\n\}", re.DOTALL)
UNIT_VARIANT = re.compile(r"^\s*(\w+),\s*$", re.MULTILINE)


def named_program_bodies(source):
    body = PROGRAM_BODY.search(source)
    if body is None:
        return ["ketchup-program model.rs: enum ProgramPartBody not found"]
    return [
        f"ProgramPartBody::{name} has no geometry of its own; describe it as a profile, "
        "path or section body"
        for name in UNIT_VARIANT.findall(body.group(1))
    ]


# A part applies its operations in the order the program wrote them; a kind of
# operation kept in a list of its own beside them (as holes and pockets were)
# is applied out of that order.
PROGRAM_OPERATION = re.compile(r"pub enum ProgramOperation \{(.*?)\n\}", re.DOTALL)
OPERATION_PAYLOAD = re.compile(r"^\s*\w+\((?:Box<)?(\w+)>?\),\s*$", re.MULTILINE)
PROGRAM_PART = re.compile(r"pub struct Part \{(.*?)\n\}", re.DOTALL)
LIST_FIELD = re.compile(r"^\s*pub (\w+): Vec<(\w+)>,\s*$", re.MULTILINE)


def operations_beside_operations(source):
    operations, part = PROGRAM_OPERATION.search(source), PROGRAM_PART.search(source)
    if operations is None or part is None:
        return ["ketchup-program model.rs: enum ProgramOperation or struct Part not found"]
    kinds = set(OPERATION_PAYLOAD.findall(operations.group(1)))
    return [
        f"Part.{field}: Vec<{kind}> keeps {kind} operations outside Part.operations; "
        "add them to the operations in program order"
        for field, kind in LIST_FIELD.findall(part.group(1))
        if kind in kinds
    ]


# The window evaluates its program through one service that keeps the result
# and warms it off the UI thread; a direct call evaluates a house program on the
# UI thread again (once per picked edge, as Fillet naming did).
PROGRAM_EVALUATION_SERVICE = "crates/ketchup-app/src/program_evaluation.rs"
DIRECT_PROGRAM_EVALUATION = re.compile(
    r"\bketchup_program::evaluate\(|\brule_program_part_sources\("
)


def direct_program_evaluations(root):
    return [
        f"{path.relative_to(root).as_posix()}:{number}: evaluate the program through "
        "KetchupApp::program_evaluations, not directly on the UI thread"
        for path in sorted((root / "crates/ketchup-app/src").glob("**/*.rs"))
        if path.relative_to(root).as_posix() != PROGRAM_EVALUATION_SERVICE
        and not is_test_module(path.relative_to(root))
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1)
        if DIRECT_PROGRAM_EVALUATION.search(line)
    ]


# Waiting for the evaluation is for answers owed in the same call only; a tool
# driven by the pointer or a key that waits freezes the window for a whole
# house evaluation after Open or Undo. Tools use try_get and show planning.
WAITING_FOR_EVALUATION = re.compile(r"\bget_blocking\(")
MAY_WAIT_FOR_EVALUATION = {
    PROGRAM_EVALUATION_SERVICE,
    "crates/ketchup-app/src/program_edit.rs",
    "crates/ketchup-app/src/live_bridge/program_access.rs",
    "crates/ketchup-app/src/live_bridge/program_pick.rs",
}


def waiting_program_evaluations(root):
    return [
        f"{path.relative_to(root).as_posix()}:{number}: a UI tool asks "
        "program_evaluations.try_get and shows planning; get_blocking is for "
        "same-call answers (live bridge, built-in assistant)"
        for path in sorted((root / "crates/ketchup-app/src").glob("**/*.rs"))
        if path.relative_to(root).as_posix() not in MAY_WAIT_FOR_EVALUATION
        and not is_test_module(path.relative_to(root))
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1)
        if WAITING_FOR_EVALUATION.search(line)
    ]


def main():
    root = Path(__file__).resolve().parents[1]
    problems = violations(workspace_dependencies(root))
    problems += oversized_modules(module_sizes(root))
    problems += oversized_functions(long_functions(root))
    problems += oversized_tests(long_tests(root))
    problems += milestone_named_tests(test_files(root))
    problems += hand_written_linear_algebra(linear_algebra_counts(root))
    problems += grown_casts_and_allows(cast_and_allow_counts(root))
    problems += direct_program_evaluations(root)
    problems += waiting_program_evaluations(root)
    model = (root / "crates/ketchup-program/src/model.rs").read_text(encoding="utf-8")
    problems += named_program_bodies(model)
    problems += operations_beside_operations(model)
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
