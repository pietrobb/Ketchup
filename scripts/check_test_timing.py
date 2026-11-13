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
  limits the work it waits for (review 2026-10-24 TST-23), and so does a call giving
  a helper that time, when the helper waits as long as its `Duration` parameter (or
  number of seconds or milliseconds) says and fails the test when it runs out, or hands
  that time on to such a helper, or sets from it a deadline after which it can fail
  (R1112-TST-500); the helper may be in any test file, a harness module included
  (review 2026-11-04 P3, r19 P3);
- a deadline under `HANG_FLOOR` after which the test can fail (`assert!(Instant::now() <
  deadline)`, an `if` past it that panics or returns `Err`, or a poll it ends followed
  by an assert or `expect`), however it is taken (`Instant::now() + X`, `start + X`,
  `.checked_add(X)`), for the same reason (review 2026-11-08 TST-252, 2026-11-09
  R1109-TST-304, 2026-11-10 R1110-TST-401);
- a call to a `perf_*` function from a function not named `perf_*`.

Not counted: a deadline that only ends a poll (`while Instant::now() < deadline`), is
followed only by an assert that the wait timed out (read from the code, not from the
words of a message: `panic!("Timeout …")` fails the test), ends a quiet wait followed
only by an assert that nothing came (`assert_eq!(count, 0)`), or fails the test only past
`HANG_FLOOR`, a wait of at least `PATIENCE_FLOOR`, and any comparison in a file that never
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
CLOCK = re.compile(r"(?:Instant|SystemTime)::now\(\)|\.elapsed\(\)")
# Unit tests inside a production file (review 2026-11-01 TST-101: functions and impls too).
from check_tolerance_literals import TEST_BUILD_MODULE as TEST_MODULE  # noqa: E402
# A comparison operator between spaces: not a generic (`Vec<u8>`), `->`, `=>` or a shift.
COMPARISON = re.compile(r"(?<=\s)(<=?|>=?)(?=\s)")
MEASURED = re.compile(
    r"\.elapsed\(\)|\.as_(?:millis|micros|nanos|secs(?:_f32|_f64)?)\(\)"
    # The same time by subtraction or `duration_since` (review 2026-10-31 TST-80).
    r"|Instant::now\(\)\s*-|\.duration_since\("
    # The checked or saturating subtraction, and the system clock (review 2026-11-01 TST-100).
    r"|\.(?:checked|saturating)_duration_since\("
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
NOW_CALL = r"(?:std::time::)?Instant::now\(\)"
BRACE = re.compile(r"[{}]")
UNIT_MS = {"secs": 1_000, "millis": 1, "micros": 1e-3, "nanos": 1e-6}
SIGNATURE = re.compile(r"\bfn\s+(\w+)\s*(?:<[^>{]*>)?\s*\(")
SELF = re.compile(r"^&?(?:'\w+\s+)?(?:mut\s+)?self\b")
DURATION_PARAMETER = re.compile(r"^(?:mut\s+)?(\w+)\s*:\s*(?:std::time::)?Duration$")
NUMBER_PARAMETER = re.compile(r"^(?:mut\s+)?(\w+)\s*:\s*(?:u8|u16|u32|u64|u128|usize)$")
NUMBER = re.compile(r"^(\d[\d_]*?)_?(?:u8|u16|u32|u64|u128|usize)?$")
# A helper's time parameter: its position, whether the helper is a method, and the
# milliseconds of one unit when the time is a number (`None` for a `Duration`).
Helper = tuple[int, bool, "float | None"]
FAILS = re.compile(r"\b(?:assert\w*|panic|unreachable)!|\.expect\(|\.unwrap\(")


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
    for brace in BRACE.finditer(text, opening):
        depth += 1 if brace.group() == "{" else -1
        if depth == 0:
            return text[opening:brace.end()]
    return text[opening:]


def duration_constants(text: str) -> dict[str, float]:
    constants = {}
    for constant in DURATION_CONST.finditer(text):
        value = milliseconds(constant.group(2), {})
        if value is not None:
            constants[constant.group(1)] = value
    return constants


def parenthesized(text: str, opening: int) -> tuple[str, int]:
    """The text inside the brackets opened at `opening`, and the index of their close."""
    depth = 0
    for index in range(opening, len(text)):
        depth += {"(": 1, ")": -1}.get(text[index], 0)
        if depth == 0:
            return text[opening + 1:index], index
    return text[opening + 1:], len(text)


def top_level_items(inner: str) -> list[str]:
    """`inner` split at the commas outside brackets."""
    items, depth, start = [], 0, 0
    for index, character in enumerate(inner):
        if character in "([{<":
            depth += 1
        elif character in ")]}>" and not inner[index - 1:index + 1] in ("->", "=>"):
            depth -= 1
        elif character == "," and depth == 0:
            items.append(inner[start:index].strip())
            start = index + 1
    tail = inner[start:].strip()
    return items + [tail] if tail else items


def signatures(text: str) -> list[tuple[str, list[str], bool, str]]:
    """Each function of `text`: its name, its parameters without `self`, whether it is a
    method (called as `.name(`) and its body."""
    found = []
    for signature in SIGNATURE.finditer(text):
        inner, close = parenthesized(text, signature.end() - 1)
        parameters = top_level_items(inner)
        method = bool(parameters) and bool(SELF.match(parameters[0]))
        found.append((signature.group(1), parameters[1:] if method else parameters, method,
                      arms(text, close)))
    return found


def calls(text: str, name: str, helper: Helper) -> list[tuple[int, str]]:
    """Each call of `helper` in `text`: where it starts and the argument holding its time."""
    position, method, _ = helper
    call = re.compile(rf"\.{name}\s*\(" if method else rf"(?<![\w.])(?<!fn ){name}\s*\(")
    found = []
    for at in call.finditer(text):
        arguments = top_level_items(parenthesized(text, at.end() - 1)[0])
        if position < len(arguments):
            found.append((at.start(), arguments[position]))
    return found


def waits_on(parameter: str, body: str, helpers: dict[str, Helper]) -> float | None | bool:
    """Whether `body` waits as long as `parameter` says and fails the test when it runs
    out: `None` for a `Duration`, the milliseconds of one unit for a number of them
    (`timeout_ms: u64` as `Duration::from_millis(timeout_ms)`), `False` if it does not."""
    typed = DURATION_PARAMETER.match(parameter)
    number = NUMBER_PARAMETER.match(parameter)
    if not typed and not number:
        return False
    name = re.escape((typed or number).group(1))
    if typed:
        unit, time = None, rf"&?{name}\b"
    else:
        converted = re.search(rf"Duration::from_(secs|millis|micros|nanos)\(\s*{name}\b", body)
        if not converted:
            return False
        unit = UNIT_MS[converted.group(1)]
        time = rf"Duration::from_{converted.group(1)}\(\s*{name}\b"
    if re.search(rf"\b(?:poll_until|recv_timeout|wait_timeout\w*|wait)\(\s*{time}", body) \
            and FAILS.search(body):
        return unit
    # A deadline it sets from its time, after which it can fail the test (review
    # 2026-11-12 R1112-TST-500: `call_within(…, 4 s)` with `assert!(Instant::now() <
    # deadline)`).
    all_deadlines = deadlines(body)
    for index, deadline in enumerate(all_deadlines):
        if re.fullmatch(time + r"\)?", (deadline.group(2) or deadline.group(3)).strip()) \
                and fails_after(body, all_deadlines, index):
            return unit
    # Its time handed on to a helper that fails when it runs out.
    for callee, helper in helpers.items():
        for _, argument in calls(body, callee, helper):
            if helper[2] is None and re.match(time + r"\)?$", argument):
                return unit
            if helper[2] is not None and number and argument == number.group(1):
                return helper[2]
    return False


def failing_wait_helpers(texts: list[str]) -> dict[str, Helper]:
    """Functions of `texts` that wait as long as one of their parameters says and fail
    the test when that time runs out, also by handing it on to such a function: name to
    that parameter's position, whether the function is a method and the unit of a number
    parameter (`None` for a `Duration`)."""
    functions = [function for text in texts for function in signatures(text)]
    helpers: dict[str, Helper] = {}
    grown = True
    while grown:
        grown = False
        for name, parameters, method, body in functions:
            if name in helpers:
                continue
            for position, parameter in enumerate(parameters):
                unit = waits_on(parameter, body, helpers)
                if unit is not False:
                    helpers[name] = (position, method, unit)
                    grown = True
                    break
    return helpers


def failing_helper_waits(text: str, helpers: dict[str, Helper],
                         constants: dict[str, float]) -> list[int]:
    """Lines of a call that gives such a helper less than `HANG_FLOOR_MS`: like a matched
    wait whose timeout fails the test, it limits the work it waits for (review
    2026-11-04 P3). The helpers and constants come from every test of the repository, so
    a harness helper counts in the files that call it (review 2026-11-04 r19 P3)."""
    constants = {**constants, **duration_constants(text)}
    found = []
    for name, helper in helpers.items():
        for start, argument in calls(text, name, helper):
            enclosing = [function.group(1) for function in FUNCTION.finditer(text, 0, start)]
            if enclosing and enclosing[-1].startswith("perf_"):
                continue  # a perf_* function may hold the work to a budget
            if helper[2] is None:
                value = milliseconds(argument, constants)
            else:
                count = NUMBER.match(argument)
                value = int(count.group(1).replace("_", "")) * helper[2] if count else None
            if value is not None and value < HANG_FLOOR_MS:
                found.append(text.count("\n", 0, start) + 1)
    return found


def short_needed_waits(text: str) -> list[int]:
    """Lines of a wait under `PATIENCE_FLOOR_MS` whose result the test relies on."""
    constants = duration_constants(text)
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


def deadlines(text: str) -> list[re.Match]:
    """Each `let deadline = Instant::now() + X;` of `text`, also from an instant taken
    before (`let start = Instant::now(); … start + X`) and by `.checked_add(X)` (review
    2026-11-10 R1110-TST-401). Group 1 is its name, group 2 or 3 its time."""
    starts = sorted({start.group(1) for start in START.finditer(text)})
    base = rf"(?:{NOW_CALL}|{'|'.join(map(re.escape, starts))})" if starts else NOW_CALL
    return list(re.finditer(
        rf"\blet\s+(?:mut\s+)?(\w+)\s*(?::\s*[\w:]+)?\s*=\s*{base}\s*(?:\+\s*([^;]+)"
        rf"|\.checked_add\(([^;]+?)\)\s*\.(?:unwrap|expect)\([^;]*\))\s*;", text))


def failing_deadlines(text: str, constants: dict[str, float]) -> list[int]:
    """Lines of a deadline under `HANG_FLOOR_MS` after which the test can fail: like a
    matched wait, it limits the work it waits for (review 2026-11-08 TST-252). It fails
    the test however it is read (`assert!(Instant::now() < deadline)`, an `if` past it
    that panics or returns `Err`, `.lt(&deadline)`), and also when a poll it ends is
    followed by an assert or `expect` on what was polled: `break false` and then
    `assert!(ok)` (review 2026-11-10 R1110-TST-401). A deadline owns the statements up
    to the next deadline or function; one that only ends a poll, or after which the test
    asserts only that the wait timed out, does not fail the test."""
    constants = {**constants, **duration_constants(text)}
    found = []
    all_deadlines = deadlines(text)
    for index, deadline in enumerate(all_deadlines):
        value = milliseconds(deadline.group(2) or deadline.group(3), constants)
        if value is None or value >= HANG_FLOOR_MS:
            continue
        if fails_after(text, all_deadlines, index):
            found.append(text.count("\n", 0, deadline.start()) + 1)
    return found


def fails_after(text: str, all_deadlines: list[re.Match], index: int) -> bool:
    """Whether the test can fail after deadline `index` of `all_deadlines`, in the
    statements it owns: up to the next deadline or function. An assert that the wait
    timed out does not count, nor, after a wait the deadline alone ends, one that
    nothing came: then the deadline is the point of the test, and a slow machine only
    waits longer (review 2026-11-12 P3: `assert_eq!(count, 0)` after a quiet wait
    counted, and the words of a message, `panic!("Timeout …")`, did not)."""
    deadline = all_deadlines[index]
    following = FUNCTION.search(text, deadline.end())
    end = following.start() if following else len(text)
    if index + 1 < len(all_deadlines):
        end = min(end, all_deadlines[index + 1].start())
    owned = text[deadline.end():end]
    quiet = re.search(rf"\bwhile\s+{NOW_CALL}\s*<\s*{re.escape(deadline.group(1))}\s*\{{", owned)
    quiet = bool(quiet) and not re.search(r"\b(?:break|return)\b", arms(owned, quiet.end() - 1))
    for statement in owned.split(";"):
        if not FAILS_AFTER.search(statement):
            continue
        words = STRING.sub('""', statement)
        if PANICS.search(words):
            return True
        if TIMED_OUT.search(words) or (quiet and NOTHING.search(words)):
            continue
        return True
    return False


START = re.compile(rf"\blet\s+(?:mut\s+)?(\w+)\s*(?::\s*[\w:]+)?\s*=\s*{NOW_CALL}\s*;")
FAILS_AFTER = re.compile(r"\b(?:assert\w*|panic|unreachable)!|\.expect\(|\bErr\(")
PANICS = re.compile(r"\b(?:panic|unreachable)!")
# A string literal, whose words say nothing of what a statement checks.
STRING = re.compile(r'"(?:[^"\\]|\\.)*"')
# A statement asserting that the wait ran out: the deadline is then the point of the test.
TIMED_OUT = re.compile(r"Timeout|TimedOut|timed_out|\.is_err\(\)|\.is_none\(\)")
# A statement asserting that nothing came: none, zero or empty.
NOTHING = re.compile(
    r"\bassert_eq!\([^;]*,\s*(?:0|None|\[\]|vec!\[\]|Vec::new\(\))\s*(?:,[^;]*)?\)\s*$"
    r"|\bassert!\(\s*[\w.()]*\.is_empty\(\)")


def budgets(text: str, helpers: dict[str, Helper], constants: dict[str, float]) -> list[int]:
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
    return sorted({*found, *short_needed_waits(text), *failing_helper_waits(text, helpers, constants),
                   *failing_deadlines(text, constants)})


def unignored_perf_tests(text: str) -> list[int]:
    """Lines of `#[test] fn perf_*` without `#[ignore`: such a test would time the PR suite."""
    return [text.count("\n", 0, test.end()) + 1 for test in PERF_TEST.finditer(text)
            if "#[test]" in test.group(1) and "#[ignore" not in test.group(1)]


def violations(root: Path = ROOT) -> list[str]:
    found = []
    tests = []
    for path in package_files(root):
        relative = path.relative_to(root)
        if "target" in relative.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        found += [f"{relative.as_posix()}:{line} perf test not ignored"
                  for line in unignored_perf_tests(text)]
        if is_test_file(relative):
            tests.append((relative, 0, text))
        else:
            # Unit tests inside production files, at their line in the file.
            tests += [(relative, text.count("\n", 0, module.start()), module.group(0))
                      for module in TEST_MODULE.finditer(text)]
    # A helper counts wherever it is called, also from another file than its own.
    helpers = failing_wait_helpers([text for _, _, text in tests])
    constants = {name: value for _, _, text in tests
                 for name, value in duration_constants(text).items()}
    for relative, offset, text in tests:
        found += [f"{relative.as_posix()}:{offset + line}"
                  for line in budgets(text, helpers, constants)]
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
