"""One failed check in a CI job does not hide the results of the checks after it."""
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
CHECK = re.compile(r"cargo (?:fmt|clippy|test)\b|pytest|run_production_tests\.py")
STILL_RUNS = ("!cancelled()", "always()", "failure()")


def jobs():
    body = WORKFLOW.split("\njobs:\n", 1)[1]
    for job in re.split(r"\n(?=  [\w-]+:\n)", body):
        name = job.split(":", 1)[0].strip()
        yield name, re.split(r"\n(?=      - )", job)[1:]


def test_every_step_after_the_first_check_runs_when_a_check_failed():
    # Review 2026-10-26 CI-44: a failed clippy or test step in the OCCT job skipped every
    # later step, so the nightly run 666 reported one failure and hid ten results.
    skipped = []
    for job, steps in jobs():
        checked = False
        for step in steps:
            condition = re.search(r"^        if: (.*)$", step, re.MULTILINE)
            if checked and not (condition and any(word in condition.group(1) for word in STILL_RUNS)):
                skipped.append(f"{job}: {step.splitlines()[0].strip()}")
            checked = checked or bool(CHECK.search(step))
    assert not skipped, "\n".join(skipped)
