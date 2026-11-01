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


def test_a_push_to_main_is_never_cancelled_by_the_next():
    """CI-61: every commit on main gets a whole run; only a pull request's
    older run gives way to its newer push."""
    concurrency = re.search(r"^concurrency:\n((?:  .*\n)+)", WORKFLOW, re.MULTILINE)
    assert concurrency, "the workflow sets concurrency"
    # A pending run is replaced by the next one in its group even without
    # cancel-in-progress, so a push has a group of its own commit.
    group = re.search(r"group:\s*(.+)", concurrency.group(1))
    assert group and group.group(1).strip().endswith(
        "${{ github.event.pull_request.number || github.sha }}"
    ), concurrency.group(1)
    cancel = re.search(r"cancel-in-progress:\s*(.+)", concurrency.group(1))
    assert cancel, concurrency.group(1)
    assert cancel.group(1).strip() == "${{ github.event_name == 'pull_request' }}", cancel.group(1)


def test_every_occt_cache_key_hashes_the_math_linked_into_it():
    """PL-63: CORE-MATH is linked into every OCCT library, so a change in
    third_party/crmath must build OCCT again on every platform."""
    keys = re.findall(r"key:\s*(occt-.+)", WORKFLOW)
    assert keys
    for key in keys:
        assert "'third_party/crmath/**'" in key, key


def test_the_windows_and_linux_occt_cache_keys_name_their_compiler():
    """Review 2026-10-31 P3: OCCT built by one compiler may not link with
    another's, so a runner image with a newer Visual Studio or clang builds it
    again. (The macOS key and Xcode are the Mac job's.)"""
    keys = {platform: key for key in re.findall(r"key:\s*(occt-.+)", WORKFLOW)
            for platform in ("windows", "ubuntu") if f"-{platform}" in key}
    assert set(keys) == {"windows", "ubuntu"}, keys
    assert "steps.vs.outputs.version" in keys["windows"], keys["windows"]
    assert "steps.clang.outputs.version" in keys["ubuntu"], keys["ubuntu"]
    clang = re.search(r"id: clang\n\s+run: \|\n((?:\s+.+\n)+)", WORKFLOW)
    assert clang and "clang --version" in clang.group(1), "the Linux job reads the clang version"


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
