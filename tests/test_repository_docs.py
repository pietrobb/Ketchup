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


def test_the_readme_does_not_place_release_only_examples_in_the_repository():
    """Review 2026-10-15 status, low: `README.md` said every example is in
    `examples/`, but some are attached only to a release."""
    text = (ROOT / "README.md").read_text(encoding="utf-8")
    assert "releases/download/" not in text or "All examples are" not in text
