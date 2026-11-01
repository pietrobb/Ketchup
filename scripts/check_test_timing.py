#!/usr/bin/env python3
"""Fail when a test outside `perf_*` asserts a wall-clock budget.

A test that asserts how long something took fails when the machine is busy (a parallel
suite, a debug build, a software GPU), not when the code is wrong. Ordinary tests check
the outcome instead: the work finished, was cancelled, or reported a timeout. A time
budget belongs only in a function named `perf_*` (the test and its helpers), marked
`#[ignore]` and run nightly in release (review 2026-10-17 TST-3).

Counted as a budget:
- a measured time held below a bound, in either order (`t.elapsed() < X`, `X > elapsed`,
  `report.cancel_ms <= 250.0`): `elapsed()`, `as_millis()` and the like, a name or field
  saying it is a time (`elapsed`, `took`, `latency`, `duration`, `*_ms`, `*_secs`, a
  percentile such as `p95` …), or a variable `let` set from one (`let spent =
  s.elapsed()`); compared by an operator (also at the start of a line of its own, as
  rustfmt breaks it), by `.lt()`/`.le()`/`.gt()`/`.ge()` or by `assert_lt!` and its kin
  (review 2026-10-24 TST-21);
- anything held below a `Duration` or a budget (`< Duration::…`, `FRAME_BUDGET >`);
- a wait shorter than `PATIENCE_FLOOR` whose result the test needs (`recv_timeout(250 ms)
  .expect(…)`), which fails when the machine is slow; polling it (`if let Ok`, `match`,
  `while`) or asserting it timed out (`.is_err()`) is not;
- a `match` on a wait whose `Timeout` arm fails the test, shorter than `HANG_FLOOR`: it
  limits the work it waits for (review 2026-10-24 TST-23);
- a call to a `perf_*` function from a function not named `perf_*`.

Not counted: a deadline that only stops a wait on a hung condition (`Instant::now() <
deadline`), a wait of at least `PATIENCE_FLOOR`, and any comparison in a file that never
reads the clock (a computed retry delay). A wait that returns as soon as its message arrives
costs a passing test nothing, so its patience can always be generous.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from source_roots import package_files  # noqa: E402
from check_test_sleeps import is_test_file  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
PATIENCE_FLOOR_MS = 5_000
# A matched wait that fails the test on its timeout limits the work it waits for;
# under this it fails on a loaded machine, so it guards only against a hang.
HANG_FLOOR_MS = 60_000
FAILS_ON_TIMEOUT = re.compile(r"Timeout\)?\s*=>\s*\{?\s*(?:panic!|unreachable!|assert)")
FUNCTION = re.compile(r"\bfn\s+(\w+)")
# A `perf_*` test and the attributes above it, which must ignore it in the PR suite.
PERF_TEST = re.compile(r"((?:^[ \t]*#\[[^\n]*\n|^[ \t]*///[^\n]*\n)+)[ \t]*fn\s+perf_\w+", re.M)
PERF_CALL = re.compile(r"(?<!fn )\bperf_\w+\s*\(")
# Code that reads the clock; elsewhere a `Duration` compared is a computed value
# (a retry delay), not a measured one.
CLOCK = re.compile(r"Instant::now\(\)|\.elapsed\(\)")
# Unit tests inside a production file, at any indentation (a `mod tests` in a `mod`), also
# under `#[cfg(all(test, windows))]` and the like.
TEST_MODULE = re.compile(
    r"^([ \t]*)#\[cfg\((?:test|(?:all|any)\([^()\n]*\btest\b[^()\n]*\))\)\][ \t]*\n(?:[ \t]*#\[[^\n]*\][ \t]*\n)*"
    r"[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod \w+ \{\n.*?^\1\}", re.M | re.S)
# A comparison operator between spaces: not a generic (`Vec<u8>`), `->`, `=>` or a shift.
COMPARISON = re.compile(r"(?<=\s)(<=?|>=?)(?=\s)")
MEASURED = re.compile(
    r"\.elapsed\(\)|\.as_(?:millis|micros|nanos|secs(?:_f32|_f64)?)\(\)"
    # The same time by subtraction or `duration_since` (review 2026-10-31 TST-80).
    r"|Instant::now\(\)\s*-|\.duration_since\("
    r"|\b[a-z_0-9]*(?:elapsed|took|latency|duration)[a-z_0-9]*\b(?!\s*\()"
    r"|\b[a-z_0-9]+_(?:ms|us|ns|millis|micros|nanos|secs|seconds)\b(?!\s*\()"
    r"|\b[a-z_0-9]*(?:p50|p90|p95|p99|p999|percentile)[a-z_0-9]*\b(?!\s*\()")
# A variable set from a measured time, which keeps measuring under its own name.
STORED = re.compile(r"\blet\s+(?:mut\s+)?(\w+)\s*(?::[^=;]+)?=([^;]+);")
# `a.lt(&b)` and its kin, and `assert_lt!(a, b)` and its kin: `lt`/`le` hold the first
# below the second, `gt`/`ge` above.
METHOD = re.compile(r"\.(lt|le|gt|ge)\(")
MACRO = re.compile(r"\b(?:debug_)?assert_(lt|le|gt|ge)!\(")
OPERATOR_FIRST = re.compile(r"^\s*(?:<=?|>=?)\s")
LIMIT = re.compile(r"\bDuration::|\b[A-Z][A-Z0-9_]*(?:BUDGET|DEADLINE)\b")
NOW = re.compile(r"^\s*(?:std::time::)?Instant::now\(\)\s*$")
WAIT = re.compile(r"\.(?:recv_timeout|wait_timeout\w*|wait|next_event)\(\s*([^()]*(?:\([^()]*\))?)\s*\)")
DURATION = re.compile(
    r"^(?:std::time::)?Duration::from_(secs|millis|micros|nanos)\((\d[\d_]*)\)$")
DURATION_CONST = re.compile(r"\bconst\s+(\w+)\s*:\s*(?:std::time::)?Duration\s*=\s*([^;]+);")
UNIT_MS = {"secs": 1_000, "millis": 1, "micros": 1e-3, "nanos": 1e-6}


def operand(text: str, backwards: bool) -> str:
    """The operand next to a comparison: up to an unbalanced bracket, `,`, `;`, `&&` or `||`."""
    depth, characters = 0, range(len(text) - 1, -1, -1) if backwards else range(len(text))
    opening, closing = ("([{", ")]}") if not backwards else (")]}", "([{")
    for index in characters:
        character = text[index]
        if character in opening:
            depth += 1
        elif character in closing:
            if depth == 0:
                return text[index + 1:] if backwards else text[:index]
            depth -= 1
        elif depth == 0 and (character in ",;" or text[index:index + 2] in ("&&", "||")
                             or (backwards and text[index - 1:index + 1] in ("&&", "||"))):
            return text[index + 1:] if backwards else text[:index]
    return text


def holds_below(small: str, large: str, stored: re.Pattern | None) -> bool:
    if NOW.match(small):
        return False  # `Instant::now() < deadline`: a wait, not a budget
    if re.search(r"\bEPSILON\b", large):
        return False  # a float tolerance (a round trip of a measured value), not a budget
    return bool(MEASURED.search(small) or LIMIT.search(large) or (stored and stored.search(small)))


def compares_a_budget(line: str, stored: re.Pattern | None = None) -> bool:
    for comparison in COMPARISON.finditer(line):
        left = operand(line[:comparison.start()], backwards=True)
        right = operand(line[comparison.end():], backwards=False)
        small, large = (left, right) if comparison.group(1).startswith("<") else (right, left)
        if holds_below(small, large, stored):
            return True
    for method in METHOD.finditer(line):
        receiver = operand(line[:method.start()], backwards=True)
        argument = operand(line[method.end():], backwards=False)
        small, large = (receiver, argument) if method.group(1) in ("lt", "le") else (argument, receiver)
        if holds_below(small, large, stored):
            return True
    for macro in MACRO.finditer(line):
        first = operand(line[macro.end():], backwards=False)
        rest = line[macro.end() + len(first) + 1:]
        second = operand(rest, backwards=False)
        small, large = (first, second) if macro.group(1) in ("lt", "le") else (second, first)
        if holds_below(small, large, stored):
            return True
    return False


def stored_measurements(text: str) -> re.Pattern | None:
    """The variables `text` sets from a measured time, as one pattern."""
    names = {name for name, value in STORED.findall(text) if MEASURED.search(value)}
    return re.compile(rf"^\W*(?:{'|'.join(map(re.escape, sorted(names)))})\b") if names else None


def milliseconds(expression: str, constants: dict[str, float]) -> float | None:
    expression = expression.strip()
    if expression in constants:
        return constants[expression]
    duration = DURATION.match(expression)
    if duration:
        return int(duration.group(2).replace("_", "")) * UNIT_MS[duration.group(1)]
    return None


def arms(text: str, after: int) -> str:
    """The braced block that follows `after`: the arms of a `match`."""
    opening = text.find("{", after)
    if opening < 0:
        return ""
    depth = 0
    for index in range(opening, len(text)):
        depth += {"{": 1, "}": -1}.get(text[index], 0)
        if depth == 0:
            return text[opening:index + 1]
    return text[opening:]


def short_needed_waits(text: str) -> list[int]:
    """Lines of a wait under `PATIENCE_FLOOR_MS` whose result the test relies on."""
    constants = {}
    for constant in DURATION_CONST.finditer(text):
        value = milliseconds(constant.group(2), {})
        if value is not None:
            constants[constant.group(1)] = value
    found = []
    for wait in WAIT.finditer(text):
        value = milliseconds(wait.group(1), constants)
        if value is None:
            continue
        start = max(text.rfind(mark, 0, wait.start()) for mark in ";{}") + 1
        ends = [end for end in (text.find(mark, wait.end()) for mark in ";{") if end >= 0]
        statement = text[start:min(ends, default=len(text))]
        if re.match(r"\s*match\b", statement) and FAILS_ON_TIMEOUT.search(arms(text, wait.end())):
            # It limits the work itself, not a message already on its way (TST-23).
            if value < HANG_FLOOR_MS:
                found.append(text.count("\n", 0, wait.start()) + 1)
            continue
        if value >= PATIENCE_FLOOR_MS:
            continue
        if re.match(r"\s*(?:if|while|match)\b", statement) or ".is_err()" in statement:
            continue  # polling, or asserting that nothing arrived in time
        found.append(text.count("\n", 0, wait.start()) + 1)
    return found


def budgets(text: str) -> list[int]:
    """Lines of `text` with a budget, or a `perf_*` call, outside functions named `perf_*`."""
    found = []
    function = ""
    clock = CLOCK.search(text)
    stored = stored_measurements(text) if clock else None
    previous = ""
    for number, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("//"):
            continue
        # rustfmt starts a broken comparison's line with its operator.
        compared = f"{previous} {line}" if OPERATOR_FIRST.match(line) else line
        previous = line
        named = FUNCTION.search(line)
        if named:
            function = named.group(1)
        if function.startswith("perf_"):
            continue
        if PERF_CALL.search(line) or (clock and compares_a_budget(compared, stored)):
            found.append(number)
    return sorted({*found, *short_needed_waits(text)})


def unignored_perf_tests(text: str) -> list[int]:
    """Lines of `#[test] fn perf_*` without `#[ignore`: such a test would time the PR suite."""
    return [text.count("\n", 0, test.end()) + 1 for test in PERF_TEST.finditer(text)
            if "#[test]" in test.group(1) and "#[ignore" not in test.group(1)]


def violations(root: Path = ROOT) -> list[str]:
    found = []
    for path in package_files(root):
        relative = path.relative_to(root)
        if "target" in relative.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        found += [f"{relative.as_posix()}:{line} perf test not ignored"
                  for line in unignored_perf_tests(text)]
        if is_test_file(relative):
            found += [f"{relative.as_posix()}:{line}" for line in budgets(text)]
        else:
            # Unit tests inside production files, at their line in the file.
            for module in TEST_MODULE.finditer(text):
                offset = text.count("\n", 0, module.start())
                found += [f"{relative.as_posix()}:{offset + line}" for line in budgets(module.group(0))]
    return found


def check(root: Path = ROOT) -> int:
    found = violations(root)
    if found:
        print("Tests assert wall-clock budgets, or a perf_* test is not ignored; assert the "
              "outcome instead, or move the budget into an ignored perf_* test run nightly in "
              "release:")
        print(*found, sep="\n  ")
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check())
