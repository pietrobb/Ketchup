"""README's build commands are the canonical ones of AGENTS.md (review 2026-10-17 DOC-1)."""
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]


def section(text: str, heading: str) -> str:
    start = text.index(heading)
    following = re.search(r"^## ", text[start + len(heading):], re.MULTILINE)
    return text[start:start + len(heading) + following.start()] if following else text[start:]


def readme_commands() -> list[str]:
    build = section((ROOT / "README.md").read_text(encoding="utf-8"), "## Build and run")
    blocks = re.findall(r"```powershell\n(.*?)```", build, re.DOTALL)
    return [line.strip() for block in blocks for line in block.splitlines() if line.strip()]


def test_readme_runs_the_definition_of_done_commands():
    done = section((ROOT / "AGENTS.md").read_text(encoding="utf-8"), "## 8. Definition of done")
    canonical = {
        command
        for command in re.findall(r"`(cargo [^`]+)`", done.replace("\n   ", " "))
        if command.startswith(("cargo clippy", "cargo test", "cargo fmt"))
    }
    assert canonical, "AGENTS.md section 8 names no cargo commands"
    commands = readme_commands()
    assert canonical <= set(commands), sorted(canonical - set(commands))


def test_readme_cargo_commands_are_locked_and_use_one_feature_set():
    for command in readme_commands():
        if command.startswith("cargo ") and not command.startswith("cargo fmt"):
            assert "--locked" in command, command
            assert "--all-features" not in command and "--features" not in command, command
            assert "--test-threads" not in command, command
