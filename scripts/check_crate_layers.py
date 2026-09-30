"""Workspace crates form layers: a crate may depend only on crates of a lower layer.

The table below is the one statement of the architecture. A new crate must be
placed in it, and a dependency that points sideways or upward fails, so the
assistant and PDM crates can never become dependencies of the model or geometry.
Only normal `[dependencies]` count; tests may use any crate.
"""

import sys
import tomllib
from pathlib import Path

LAYERS = {
    "ketchup-tolerance": 0,
    "ketchup-core": 1,
    "ketchup-exact": 1,
    "ketchup-interaction": 2,
    "ketchup-assistant": 2,
    "ketchup-program": 3,
    "ketchup-scheduler": 3,
    "ketchup-application": 4,
    "ketchup-headless": 5,
    "ketchup-app": 5,
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


def main():
    root = Path(__file__).resolve().parents[1]
    problems = violations(workspace_dependencies(root))
    for problem in problems:
        print(problem)
    if not problems:
        print("OK")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
