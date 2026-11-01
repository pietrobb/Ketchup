"""`docs/` holds only normative English documents and ADRs (review REPO-2, structural point 9).

Dated reviews, status files and working notes go to `docs/archive/`, where they are history
and not binding. ADR 0001 makes English the language of documentation, so a Slovak document
outside the archive is a decision that has not been carried into an English ADR yet."""

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DATED = re.compile(r"\d{4}-\d{2}-\d{2}")
# Letters that appear in Slovak and not in English text; the product's Slovak name is allowed.
SLOVAK_LETTERS = re.compile("[áäčďéíĺľňóôŕšťúýžÁÄČĎÉÍĹĽŇÓÔŔŠŤÚÝŽ]")
PRODUCT_NAME = re.compile(r"Kečup\w*")


def current_docs():
    listed = subprocess.run(
        ["git", "ls-files", "-z", "--", "docs"], cwd=ROOT, capture_output=True, check=True
    ).stdout.decode("utf-8")
    return [name for name in listed.split("\0") if name and not name.startswith("docs/archive/")]


def test_dated_reviews_and_notes_live_in_the_archive():
    dated = [name for name in current_docs() if DATED.search(Path(name).name)]
    assert dated == [], "move dated reviews, status files and notes to docs/archive/"


def test_current_documents_are_english():
    slovak = {}
    for name in current_docs():
        if Path(name).suffix != ".md":
            continue
        text = PRODUCT_NAME.sub("", (ROOT / name).read_text(encoding="utf-8"))
        letters = len(SLOVAK_LETTERS.findall(text))
        if letters:
            slovak[name] = letters
    assert slovak == {}, "write normative documents in English (ADR 0001)"


def test_the_entry_documents_link_only_to_documents_that_exist():
    missing = []
    for entry in ("README.md", "AGENTS.md"):
        text = (ROOT / entry).read_text(encoding="utf-8")
        for link in sorted(set(re.findall(r"docs/[\w./-]+\.(?:md|html)", text))):
            if not (ROOT / link).is_file():
                missing.append(f"{entry}: {link}")
    assert missing == []


MARKDOWN_LINK = re.compile(r"\]\(([^)\s#]+)(?:#[^)]*)?\)")


def test_relative_links_of_current_documents_lead_to_tracked_files():
    """Review 2026-10-15 status, low: dead paths in ADRs 0006 and 0007 and in
    `viewer/web/HOSTING.md`. A link is followed from the document's folder."""
    tracked = set(
        subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True)
        .stdout.decode("utf-8")
        .split("\0")
    )
    folders = {str(Path(name).parent).replace("\\", "/") for name in tracked}
    documents = [
        name for name in current_docs() if name.endswith(".md")
    ] + ["README.md", "AGENTS.md", "viewer/web/HOSTING.md"]
    missing = []
    for document in documents:
        text = (ROOT / document).read_text(encoding="utf-8")
        for link in sorted(set(MARKDOWN_LINK.findall(text))):
            if re.match(r"[a-z]+:", link):
                continue
            target = (ROOT / document).parent.joinpath(link).resolve()
            try:
                relative = target.relative_to(ROOT).as_posix()
            except ValueError:
                missing.append(f"{document}: {link}")
                continue
            if relative not in tracked and relative.rstrip("/") not in folders:
                missing.append(f"{document}: {link}")
    assert missing == []


QUOTED_PATH = re.compile(
    r"`([\w.-]+(?:/[\w.-]+)+/?|[\w-]+\.(?:md|rs|cc|h|json|yaml|toml|py))`"
)


def test_the_files_an_adr_names_exist():
    """Review 2026-10-17, documentation: ADRs 0002 and 0004 cite evidence files that are
    not in the repository. A quoted path or file name must be a tracked file or folder,
    named from the repository root or by its own name."""
    tracked = set(
        subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, capture_output=True, check=True)
        .stdout.decode("utf-8")
        .split("\0")
    )
    folders = {str(Path(name).parent).replace("\\", "/") for name in tracked}
    names = {Path(name).name for name in tracked}
    missing = []
    for adr in sorted((ROOT / "docs" / "adr").glob("*.md")):
        for path in sorted(set(QUOTED_PATH.findall(adr.read_text(encoding="utf-8")))):
            path = path.rstrip("/")
            if path not in tracked and path not in folders and path not in names:
                missing.append(f"{adr.name}: {path}")
    assert missing == []


def test_every_adr_has_its_own_number():
    """Review 2026-10-29 DOC-61: two ADRs were numbered 0009, so "ADR 0009" named two
    decisions. The file name and the title carry the same number, and no number repeats."""
    numbers = {}
    for adr in sorted((ROOT / "docs" / "adr").glob("*.md")):
        number = adr.name.split("-", 1)[0]
        title = adr.read_text(encoding="utf-8").splitlines()[0]
        assert title.startswith(f"# ADR {number}:"), f"{adr.name}: {title}"
        numbers.setdefault(number, []).append(adr.name)
    assert {n: names for n, names in numbers.items() if len(names) > 1} == {}


def test_identical_results_across_systems_cite_the_math_adr():
    """Review 2026-10-31 P3: the README, CI, the Linux OCCT build and the shop
    golden test cited ADR 0009 (rule programs) for the promise of the same bits on
    every system, which ADR 0010 (correctly rounded math) makes."""
    tracked = (
        subprocess.run(["git", "ls-files", "-z"], cwd=ROOT, check=True, capture_output=True)
        .stdout.decode("utf-8")
        .split("\0")
    )
    topic = re.compile(r"crmath|CORE-MATH|bit for bit|same bytes|same files", re.IGNORECASE)
    wrong = []
    for name in tracked:
        if not name or name.startswith(("docs/archive/", "docs/adr/")):
            continue
        path = ROOT / name
        if path.suffix not in {".md", ".rs", ".py", ".sh", ".ps1", ".yml", ".toml"}:
            continue
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
        for index, line in enumerate(lines):
            if "ADR 0009" in line and topic.search(" ".join(lines[max(0, index - 3) : index + 2])):
                wrong.append(f"{name}:{index + 1}")
    assert wrong == []


def test_the_readme_states_the_import_limit_of_0_2_0_the_code_keeps():
    """Review 2026-10-31 P3: the README said 0.2.0 reads at most 341 profiles from a
    drawing, while the format-103 check lets an import keep 1 024 outputs."""
    codec = (
        ROOT / "crates" / "ketchup-model" / "src" / "persistence" / "snapshot_codec.rs"
    ).read_text(encoding="utf-8")
    outputs = int(re.search(r"const OUTPUTS: usize = ([\d_]+);", codec).group(1).replace("_", ""))
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    assert f"at most {outputs:,}".replace(",", " ") + " objects made by one import" in readme


def test_the_readme_does_not_place_release_only_examples_in_the_repository():
    """Review 2026-10-15 status, low: `README.md` said every example is in
    `examples/`, but some are attached only to a release."""
    text = (ROOT / "README.md").read_text(encoding="utf-8")
    assert "releases/download/" not in text or "All examples are" not in text


def test_the_reference_documents_what_kecup_writes_into_a_program():
    """Review 2026-10-26 DOC-40: Kečup writes `profile_part()` and
    `instance(matrix=)` into the user's program from a drawing, and the library
    offers `connector()` and the build-up keys `sheet` and `grain`, but the
    language reference named none of them; the README said a drawing never
    becomes parts and left out its 2D edits, export and MCP `drawing` actions."""
    generator = (
        ROOT / "crates" / "ketchup-app" / "src" / "live_bridge" / "underlay_program.rs"
    ).read_text(encoding="utf-8")
    written = set(re.findall(r"\b(profile_part|instance|component|extrude|cut)\(", generator))
    reference = (ROOT / "docs" / "program-language.md").read_text(encoding="utf-8")
    undocumented = sorted(name for name in written if f"`{name}(" not in reference)
    for needed in ["matrix=", "`connector(", '`"sheet"', '`"grain"', "# underlay binding"]:
        if needed not in reference:
            undocumented.append(needed)
    assert undocumented == []
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    assert "without turning it into parts" not in readme
    for needed in ["Export drawing to DXF", "Pull drawing area up", "edit it (remove, draw"]:
        assert needed in readme, needed
