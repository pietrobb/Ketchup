"""A CI job that lints its crates with features on also lints them as AGENTS.md does, without."""
import re

from test_ci_steps_after_failure import jobs

CLIPPY = re.compile(r"cargo clippy\b[^\n]*")


def test_a_job_linting_with_features_also_lints_without_them():
    # Review 2026-11-04 P3: the crates without OCCT were linted only with
    # `--features`, so code behind `cfg(not(feature))` was not linted in any job.
    missing = []
    for job, steps in jobs():
        runs = [CLIPPY.search(step).group(0) for step in steps if CLIPPY.search(step)]
        for run in runs:
            if "--features" in run:
                plain = re.sub(r" --features \S+", "", run)
                if plain not in runs:
                    missing.append(f"{job}: {plain}")
    assert not missing, "\n".join(missing)
