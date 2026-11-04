"""Every package denies the clippy lints that see a dropped error (review 2026-11-04 TST-140).

A package with its own `[lints.clippy]` table does not inherit the workspace one, so
each such table has to name them too; scripts/check_error_hygiene.py keeps only what
clippy cannot see.
"""
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DROP_LINTS = ("map_err_ignore", "let_underscore_must_use", "let_underscore_untyped")


def manifests():
    for pattern in ("crates/*/Cargo.toml", "viewer/*/Cargo.toml"):
        yield from sorted(ROOT.glob(pattern))


def denied(table):
    return [lint for lint in DROP_LINTS if table.get(lint) not in ("deny", "forbid")]


def test_the_workspace_denies_the_drop_lints():
    workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    assert denied(workspace["workspace"]["lints"]["clippy"]) == []


def test_every_package_denies_the_drop_lints():
    missing = {}
    for manifest in manifests():
        lints = tomllib.loads(manifest.read_text(encoding="utf-8")).get("lints")
        relative = manifest.relative_to(ROOT).as_posix()
        if lints is None:
            missing[relative] = "no [lints] table"
        elif lints.get("workspace") is not True:
            absent = denied(lints.get("clippy", {}))
            if absent:
                missing[relative] = absent
    assert missing == {}
