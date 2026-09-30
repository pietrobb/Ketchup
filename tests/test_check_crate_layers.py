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
        assert layers[crate] > layers["ketchup-core"]
        assert crate not in graph["ketchup-core"]


def test_geometry_depends_on_nothing_but_the_tolerances():
    graph = checker.workspace_dependencies(ROOT)
    assert graph["ketchup-geometry"] == ["ketchup-tolerance"]
    assert checker.LAYERS["ketchup-geometry"] < checker.LAYERS["ketchup-core"]
