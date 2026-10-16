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
