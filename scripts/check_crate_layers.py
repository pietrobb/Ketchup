"""Workspace crates form layers: a crate may depend only on crates of a lower layer.

The table below is the one statement of the architecture. A new crate must be
placed in it, and a dependency that points sideways or upward fails, so the
assistant and PDM crates can never become dependencies of the model or geometry.
Only normal `[dependencies]` count; tests may use any crate.

No source module of any crate may exceed MAX_MODULE_LINES. Modules that were
already larger are listed in OVERSIZED with their size: they may shrink but not
grow, and an entry must be removed once its module fits the limit.
"""

import sys
import tomllib
from pathlib import Path

LAYERS = {
    "ketchup-tolerance": 0,
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
    "crates/ketchup-app/src/lib.rs": 37_980,
    "crates/ketchup-app/src/tests.rs": 18_330,
}


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


def main():
    root = Path(__file__).resolve().parents[1]
    problems = violations(workspace_dependencies(root))
    problems += oversized_modules(module_sizes(root))
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
