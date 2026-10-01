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
    "ketchup-headless": 6,
    "ketchup-app": 6,
}

MAX_MODULE_LINES = 5_000

OVERSIZED = {
    "crates/ketchup-app/src/tests.rs": 17_967,
}

MAX_FUNCTION_LINES = 400

# Production functions that were already longer, with their length. They may
# shrink but not grow; an entry is removed once its function fits the limit.
LONG_FUNCTIONS = {
    "crates/ketchup-app/src/assembly_ui.rs::show_assembly_editor_content": 756,
    "crates/ketchup-app/src/feature_history_ui.rs::show_feature_history_content": 672,
    "crates/ketchup-application/src/append_feature.rs::plan_feature_kind": 794,
    "crates/ketchup-application/src/collision.rs::collision_report": 853,
    "crates/ketchup-application/src/creation.rs::plan_creation": 411,
    "crates/ketchup-application/src/planner.rs::plan_assistant_cad_edit_program_with_outputs": 2027,
    "crates/ketchup-application/src/validation.rs::assistant_assembly_retention_report": 455,
    "crates/ketchup-application/src/validation.rs::assistant_hardware_manufacturing_report": 432,
    "crates/ketchup-application/src/validation.rs::assistant_validation_context_base": 653,
    "crates/ketchup-assistant/src/intent.rs::propose_intent": 1011,
    "crates/ketchup-assistant/src/sidecar.rs::validate": 733,
    "crates/ketchup-model/src/document/digest_v3.rs::feature_kind": 702,
    "crates/ketchup-model/src/document/feature_validation.rs::validate_feature_kind": 408,
    "crates/ketchup-model/src/document/product_validation.rs::validate_product_with_drawing_sources": 1132,
    "crates/ketchup-model/src/document/proposal_analysis.rs::authoritative_dependencies": 969,
    "crates/ketchup-model/src/document/solid_tool.rs::clone_definition_and_repoint": 492,
    "crates/ketchup-model/src/document/store.rs::apply_batch_with_origin_and_validation": 2141,
    "crates/ketchup-model/src/exact_brep_graph/compiler.rs::compile_node": 615,
    "crates/ketchup-model/src/persistence/legacy.rs::read_product": 1396,
    "crates/ketchup-model/src/shared_change.rs::commit_component_replacement": 430,
    "crates/ketchup-model/src/shared_change.rs::commit_occurrence_fork_change": 572,
    "crates/ketchup-model/src/shared_change.rs::project_component_replacement_impact_for_principal": 823,
    "crates/ketchup-model/src/shared_change.rs::project_occurrence_fork_impact": 484,
    "crates/ketchup-program/src/eval.rs::builtins": 1018,
    "crates/ketchup-scheduler/src/exact_worker.rs::evaluate_exact_brep_graph": 559,
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


def long_functions(root, limit=MAX_FUNCTION_LINES):
    """{"crates/<crate>/src/...rs::name": lines} for production functions over the limit."""
    return {
        f"{path.relative_to(root).as_posix()}::{name}": length
        for path in sorted((root / "crates").glob("*/src/**/*.rs"))
        if "tests" not in path.stem
        for name, length in function_lengths(path).items()
        if length > limit
    }


def oversized_functions(lengths, recorded=LONG_FUNCTIONS, limit=MAX_FUNCTION_LINES):
    problems = []
    for function, length in lengths.items():
        allowed = recorded.get(function, limit)
        if length > allowed:
            problems.append(
                f"{function}: {length} lines, limit {allowed}; split it into named steps"
            )
    for function, length in recorded.items():
        if function not in lengths:
            problems.append(f"{function}: fits {limit} lines now; remove it from LONG_FUNCTIONS")
        elif lengths[function] < length:
            problems.append(
                f"{function}: shrank to {lengths[function]} lines; lower LONG_FUNCTIONS to it"
            )
    return problems


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
)
# The remaining hand-written spots per file; the counts may only fall.
LINEAR_ALGEBRA = {
    "crates/ketchup-analysis/src/fea.rs": 1,
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


def main():
    root = Path(__file__).resolve().parents[1]
    problems = violations(workspace_dependencies(root))
    problems += oversized_modules(module_sizes(root))
    problems += oversized_functions(long_functions(root))
    problems += milestone_named_tests(test_files(root))
    problems += hand_written_linear_algebra(linear_algebra_counts(root))
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
