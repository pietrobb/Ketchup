"""THIRD_PARTY_NOTICES.txt of a Kecup package (review 2026-11-01 REL-100).

Each package names what its programs contain, and only that: the components
with conditions of their own (scripts/notices/*.txt), then every Rust crate
compiled into its programs with its copyright notice and license text, as MIT,
BSD and ISC require of a binary distribution. cargo-about lists the crates and
their license texts for the package's programs and target; cargo tree checks
that the list is complete and that a component section is there exactly when
its crate is compiled in.

    python scripts/third_party_notices.py --package viewer --output DIR
    python scripts/third_party_notices.py --check   # every package, nothing written
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NOTICES = ROOT / "scripts" / "notices"
TARGET = "x86_64-pc-windows-msvc"
# License texts that are an attribution requirement: the copyright notice must
# travel with the binary.
ATTRIBUTION = {"MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC"}


@dataclass(frozen=True)
class Section:
    name: str
    # The crate whose presence in a program brings the component in.
    crate: str


SECTIONS = {
    "occt": Section("occt", "ketchup-exact"),
    "acadrust": Section("acadrust", "acadrust"),
    "starlark": Section("starlark", "starlark"),
    "core-math": Section("core-math", "ketchup-exact"),
}


@dataclass(frozen=True)
class Package:
    title: str
    # Workspace packages whose programs the package carries.
    programs: tuple[str, ...]
    sections: tuple[str, ...]


PACKAGES = {
    "ketchup": Package(
        "Kecup",
        # The installer and the portable archive carry the Viewer too.
        ("ketchup-app", "ketchup-scheduler", "ketchup-viewer"),
        ("occt", "acadrust", "starlark", "core-math"),
    ),
    "viewer": Package("Kecup Viewer", ("ketchup-viewer",), ()),
}


class NoticeError(Exception):
    pass


def run(args: list[str]) -> str:
    result = subprocess.run(
        args, cwd=ROOT, capture_output=True, text=True, encoding="utf-8"
    )
    if result.returncode != 0:
        raise NoticeError(f"{' '.join(args)} failed:\n{result.stderr.strip()}")
    return result.stdout


def about(program: str, target: str, offline: bool) -> dict:
    with tempfile.TemporaryDirectory() as directory:
        output = Path(directory) / "about.json"
        args = ["cargo", "about", "generate", "--format", "json", "--locked"]
        args += ["--target", target, "-m", f"crates/{program}/Cargo.toml"]
        args += ["-o", str(output)] + (["--offline"] if offline else [])
        run(args)
        return json.loads(output.read_text(encoding="utf-8"))


def tree(program: str, target: str) -> set[tuple[str, str, bool]]:
    """(name, version, from a registry) of every crate the program compiles in."""
    out = run(
        ["cargo", "tree", "--locked", "-p", program, "-e", "normal", "--prefix", "none"]
        + ["--target", target, "--format", "{p}"]
    )
    crates = set()
    for line in out.splitlines():
        match = re.match(r"(\S+) v(\S+)( \(.*?\))?", line.strip())
        if match:
            crates.add((match[1], match[2], match[3] is None or "registry" in match[3]))
    return crates


def merge(reports: list[dict]) -> list[dict]:
    """Distinct license texts, each with the crates it covers."""
    texts: dict[tuple[str, str], dict] = {}
    for report in reports:
        for license in report["licenses"]:
            key = (license["id"], license["text"])
            entry = texts.setdefault(
                key,
                {"id": license["id"], "name": license["name"], "text": license["text"], "crates": {}},
            )
            for use in license["used_by"]:
                crate = use["crate"]
                entry["crates"][(crate["name"], crate["version"])] = crate
    return sorted(
        texts.values(),
        key=lambda entry: (entry["id"], min(entry["crates"]), entry["text"]),
    )


def has_copyright(text: str) -> bool:
    return re.search(r"copyright|\(c\)|©", text, re.IGNORECASE) is not None


def verify(package: Package, licenses: list[dict], compiled: set[tuple[str, str, bool]]) -> None:
    listed = {key for entry in licenses for key in entry["crates"]}
    missing = sorted(
        f"{name} {version}"
        for name, version, registry in compiled
        if registry and (name, version) not in listed
    )
    if missing:
        raise NoticeError(f"{package.title}: no license text for {', '.join(missing)}")
    for entry in licenses:
        if entry["id"] not in ATTRIBUTION or has_copyright(entry["text"]):
            continue
        anonymous = sorted(
            f"{name} {version}"
            for (name, version), crate in entry["crates"].items()
            if not crate.get("authors")
        )
        if anonymous:
            raise NoticeError(
                f"{package.title}: the {entry['id']} text of {', '.join(anonymous)} "
                "names no copyright holder and the crates no authors"
            )
    names = {name for name, _, _ in compiled}
    for name, section in SECTIONS.items():
        present = section.crate in names
        if present != (name in package.sections):
            state = "compiles in" if present else "does not compile in"
            raise NoticeError(
                f"{package.title} {state} {section.crate}, so its {name} section "
                f"must be {'there' if present else 'left out'}"
            )


def crate_line(crate: dict) -> str:
    line = f"  {crate['name']} {crate['version']}"
    if crate.get("authors"):
        line += " - " + ", ".join(crate["authors"])
    if crate.get("repository"):
        line += f"\n    {crate['repository']}"
    return line


def render(package: Package, licenses: list[dict]) -> str:
    def heading(text: str) -> list[str]:
        return [text, "-" * len(text)]

    title = f"{package.title} - third-party notices"
    lines = [title, "=" * len(title), ""]
    if package.sections:
        lines += [
            f"{package.title} includes the components below, each with its",
            "license; the license texts named ship beside this file. After them",
            "come the Rust crates compiled into its programs, each with its",
            "copyright notice and license text.",
        ]
    else:
        lines += [
            f"{package.title} is made of the Rust crates below, compiled into its",
            "program; each is listed with its copyright notice and license text.",
        ]
    number = 0
    for name in package.sections:
        number += 1
        title, _, body = (NOTICES / f"{name}.txt").read_text(encoding="utf-8").partition("\n")
        lines += ["", ""] + heading(f"{number}. {title}") + [body.strip("\n")]
    number += 1
    lines += ["", ""] + heading(f"{number}. Rust crates")
    lines += [
        "Each license text follows the crates it covers, with their authors",
        "and source.",
    ]
    for entry in licenses:
        lines += ["", "", f"{entry['name']} ({entry['id']})", "Used by:"]
        lines += [crate_line(entry["crates"][key]) for key in sorted(entry["crates"])]
        lines += ["", entry["text"].strip("\n")]
    return "\n".join(line.rstrip() for line in "\n".join(lines).split("\n")) + "\n"


def notices(name: str, target: str = TARGET, offline: bool = False) -> str:
    package = PACKAGES[name]
    licenses = merge([about(program, target, offline) for program in package.programs])
    compiled = set().union(*(tree(program, target) for program in package.programs))
    verify(package, licenses, compiled)
    return render(package, licenses)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--package", choices=sorted(PACKAGES))
    parser.add_argument("--output", type=Path, help="folder the file is written to")
    parser.add_argument("--check", action="store_true", help="generate every package, write nothing")
    parser.add_argument("--target", default=TARGET)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args(argv)
    if args.check == (args.package is not None) or (args.package is None) != (args.output is None):
        parser.error("give --check, or --package with --output")
    try:
        if args.check:
            for name in PACKAGES:
                notices(name, args.target, args.offline)
                print(f"OK: {PACKAGES[name].title} notices name every crate it compiles in")
        else:
            text = notices(args.package, args.target, args.offline)
            args.output.mkdir(parents=True, exist_ok=True)
            (args.output / "THIRD_PARTY_NOTICES.txt").write_text(text, encoding="utf-8", newline="\r\n")
    except NoticeError as error:
        print(f"third_party_notices: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
