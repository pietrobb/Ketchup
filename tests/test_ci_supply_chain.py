"""CI runs only pinned code: actions by commit, Python packages by hash, and
cargo-deny checks the dependencies for every platform something is built for."""
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")


def test_every_action_is_pinned_to_a_commit():
    uses = re.findall(r"^\s*-?\s*uses:\s*(\S+)", WORKFLOW, re.MULTILINE)
    assert uses
    for action in uses:
        assert re.fullmatch(r"[\w.-]+/[\w./-]+@[0-9a-f]{40}", action), action


def test_python_packages_install_only_by_hash():
    installs = [line for line in WORKFLOW.splitlines() if "pip install" in line]
    assert installs
    for line in installs:
        assert "--require-hashes -r tests/requirements-ci.txt" in line, line
    requirements = (ROOT / "tests" / "requirements-ci.txt").read_text(encoding="utf-8")
    packages = re.findall(r"^([\w.-]+)==\S+", requirements, re.MULTILINE)
    assert {"pytest", "openpyxl"} <= set(packages)
    for block in re.split(r"\n(?=[\w.-]+==)", requirements.split("\n", 2)[2]):
        assert "--hash=sha256:" in block, block


def test_cargo_deny_runs_a_verified_release_for_all_targets():
    assert re.search(r"sha256sum --check", WORKFLOW)
    assert re.search(r"^\s*run: cargo deny --locked check\s*$", WORKFLOW, re.MULTILINE)
    assert re.search(r"needs: \[[^\]]*\bdependencies\b", WORKFLOW)
    deny = tomllib.loads((ROOT / "deny.toml").read_text(encoding="utf-8"))
    assert set(deny["graph"]["targets"]) >= {
        "x86_64-pc-windows-msvc",
        "x86_64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "aarch64-linux-android",
        "wasm32-unknown-unknown",
    }
    for ignored in deny["advisories"].get("ignore", []):
        assert ignored.get("reason"), ignored
