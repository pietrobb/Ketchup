#!/usr/bin/env python3
"""Fail when crate source gains an error that throws away its cause (AGENTS.md §2).

Two forms are counted in production source (crates/*/src, test modules and
*tests.rs files excluded):

- discard: `map_err(|_| ...)` or `map_err(|_name| ...)` drops the error it
  receives. Only an error type that carries no content may be dropped, and the
  closure must name it: `map_err(|_: std::num::TryFromIntError| ...)`. The
  content-free types are listed in CONTENT_FREE; any other typed discard counts.
- string: `Err("...".to_owned())`, `.to_string()` or `.into()` answers with bare
  text instead of a typed error or `ketchup_rejection::Rejection`.
- unreachable: `unreachable!(...)` admits that a type allows a case the code
  says cannot happen; shape the type so the case does not exist instead.
- silent_unreachable: `unreachable!()` with no message also hides which
  assumption broke when it fires.
- string_result: `Result<_, String>` (or `Ok::<_, String>`) makes every caller unable to tell one
  failure from another.
- str_result: `Result<_, &'static str>` (or any `&str`) is the same text error as a
  constant (review 2026-10-29 TST-43: it rose from 166 to 179 unseen).
- stringified_cause: `map_err(|e| e.to_string())` or `map_err(ToString::to_string)`
  flattens a typed error into text, so the caller can no longer react to its kind.
- debug_formatted: `format!("{error:?}")` or `format!("{:?}", error)` turns an
  error into its Debug text, which neither the caller nor the user can act on.
- stringified_variant: `Variant(error.to_string())` does the same inside an
  error variant, and `Error::new(code, error.to_string())` inside a constructor
  function; carry the source error as a field instead.
- formatted_variant: `Variant(format!(...))` puts the facts of an error into a
  sentence instead of fields (`Some`, `Ok` and `Err` are not error variants;
  `Err(format!)` is already a string_result).
- unwrap: `.unwrap()` panics without saying which assumption broke.
- dropped_variant: `SomeError::Variant(_) => "code"` answers an error that carries
  a cause with a bare code and throws the cause away; record it
  (`failed_because(code, cause)`) or map the variant to a typed error.
- dropped_error: `map_err(drop)`, `if let Err(_) = ...` and `.ok()?` drop the error
  as `map_err(|_| ...)` does (review 2026-10-31 TST-80).
- formatted_cause: `format!("{e}")` or `format!("{}", error)` turns an error into its
  Display text in place of the error.
- stringified_field: `Error::Io { message: e.to_string() }` does it inside a struct
  literal, as stringified_variant does inside a tuple variant.
- let_else_error: `let Ok(x) = f() else { ... }` leaves the error out of the branch that
  answers it (review 2026-11-01 TST-100); `match` it, or `map_err` it with a typed
  closure when it carries nothing. A text check cannot tell a content-free error here.
- dropped_quietly: `.or_else(|_| ...)` and `Err(_) => None` drop the error as
  `map_err(|_| ...)` does.
- dropped_arm: any other arm `Err(_) =>` or `Err(_name) =>`, such as
  `Err(_) => { return Err(..) }` (review 2026-11-02 TST-121).
- dropped_closure: `.unwrap_or_else(|_| ...)` answers an error with a value.
- let_underscore: `let _ = call(...);` throws away what the call answers; clippy's
  `let_underscore_must_use` sees the same where a crate holds none (TST-121).
- guarded_arm: `Err(_) if cond =>` drops the error the guard decides on.
- named_underscore: `let _name = call();` keeps what the call answers unread; a
  guard held to the end of the scope (`_lock`, `_guard`, `_permit`, `_reset`,
  `_restore`) is exempt.
- ok_statement: a bare `.ok();` (or `.ok().is_some();`) throws the error away as
  `let _ =` does.
- from_discard: `fn from(_: E)` turns an error into another without its content.
- empty_is_err: `if call().is_err() { /* comment */ }` does what `let _ =` does
  (review 2026-11-04 r19 TST-160).
- irrefutable_let: `let (Ok(()) | Err(..)) = call();` does it too.
- dropped_call: `drop(call())` does it too (review 2026-11-06 P3).
- or_default: `.unwrap_or_default()` answers an error with a default value; a text
  check cannot tell an Option's, so each one counts.
- eprintln: `eprintln!` outside a command line's entry point (`src/main.rs`,
  `src/bin/`), or in one after its window logs to a file (`log_window_to`),
  writes where a window started without a console throws it away.
  `ketchup_rejection::dropped(result, Reason::...)` is the one way to drop an
  error, and `ketchup_rejection::log!` the way to log one; both write to the
  window's log file.

clippy's map_err_ignore, let_underscore_must_use and let_underscore_untyped
(workspace [lints]) cover the rest; these four are what clippy cannot see
(review 2026-11-04 TST-140).

The check is a ratchet: scripts/error_hygiene_baseline.txt records what still
exists, per file and form. A count may only go down; a new file or a higher count
fails. Run with --update after removing occurrences to shrink the baseline; it never
grows the count of a form the baseline lists in its `# forms:` line. A form added to
this check is recorded as found by the first --update after it, once. Never add
entries by hand: the baseline and the checks are owned in .github/CODEOWNERS.

The viewer apps (viewer/*/src) are production source too.
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from source_roots import package_files  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
BASELINE_NAME = "scripts/error_hygiene_baseline.txt"
EXCLUDED_PARTS = {"tests", "examples", "fixtures"}
# Errors whose value says nothing beyond their type: dropping them loses nothing.
# An entry matches the named type by its trailing path, so "fmt::Error" accepts
# `std::fmt::Error` but not an unrelated `io::Error`.
CONTENT_FREE = {
    "TryFromIntError", "TryFromSliceError", "PoisonError", "TryLockError", "SendError",
    "TrySendError", "RecvError", "TryRecvError", "RecvTimeoutError", "Infallible",
    "fmt::Error",
    # A float parse failure only says the token was empty or not a number.
    "ParseFloatError",
    # An integer parse failure only says the digits were empty, invalid or out of range.
    "ParseIntError",
    # Says only that the path does not start with the prefix.
    "StripPrefixError",
    # wgpu reports a failed buffer map as a unit struct.
    "wgpu::BufferAsyncError",
    # Opaque by design: a failed signature check must not reveal why it failed.
    "ed25519_dalek::SignatureError",
}
DISCARD = re.compile(r"map_err\(\s*(?:move\s*)?\|\s*_\w*\s*(?::\s*([^|]+?)\s*)?\|")
# The same drop without map_err (review 2026-10-26 TST-43): `.ok().ok_or(E)`,
# `.or(Err(E))` and a match arm `Err(_) => Err(E)` / `Err(_) => return Err(E)`.
DISCARD_OTHERWISE = re.compile(
    r"\.ok\(\)\s*\.\s*ok_or(?:_else)?\(|\.or\(\s*Err\(|\bErr\(\s*_\s*\)\s*=>\s*(?:return\s+)?Err\(")
STRING = re.compile(r"""Err\(\s*"(?:[^"\\]|\\.)*"\s*\.\s*(?:to_owned|to_string|into)\(\)\s*\)""")
UNREACHABLE = re.compile(r"\bunreachable!\s*\(")
SILENT_UNREACHABLE = re.compile(r"\bunreachable!\s*\(\s*\)")
# `Result<…, E>`, or the turbofish `Ok::<_, E>(...)` naming the same error type; the
# brackets are followed to the end, so any Ok type (generic, tuple, closure) is seen.
RESULT_OPEN = re.compile(r"\b(?:Result|(?:Ok|Err)::)<")
STR_ERROR = re.compile(r"&\s*(?:'\w+\s+)?str")
STRINGIFIED_CAUSE = re.compile(
    r"map_err\(\s*(?:(?:move\s*)?\|\s*(\w+)\s*\|\s*\1\s*\.\s*to_string\(\)"
    r"|(?:std::string::)?ToString::to_string)\s*\)")
ERROR_NAME = r"(?:e|err|error|\w+_err(?:or)?|source|cause)"
DEBUG_FORMATTED = re.compile(
    r'format!\(\s*"(?:[^"\\]|\\.)*?(?:\{' + ERROR_NAME + r':#?\?\}'
    r'|\{:#?\?\}(?:[^"\\]|\\.)*"\s*,\s*&?' + ERROR_NAME + r'\b)')
# Also a constructor function of a type, `Error::new(code, error.to_string())` or
# `Error::invalid(e.to_string())`: the same text in place of the source error.
STRINGIFIED_VARIANT = re.compile(
    r"\b(?!(?:Some|Ok|Err)\()(?:[A-Z]\w*|[A-Z]\w*::[a-z_]\w*)\(\s*"
    r"(?:(?:\"(?:[^\"\\]|\\.)*\"|[\w.]+(?:\(\))?)\s*,\s*)?&?\b" + ERROR_NAME
    + r"\s*\.\s*to_string\(\)\s*\)")
FORMATTED_VARIANT = re.compile(r"\b(?!(?:Some|Ok|Err)\()[A-Z]\w*\(\s*format!\(")
UNWRAP = re.compile(r"\.unwrap\(\)")
DROPPED_ERROR = re.compile(r"map_err\(\s*drop\s*\)|\bif\s+let\s+Err\(\s*_\s*\)\s*=|\.ok\(\)\s*\?")
FORMATTED_CAUSE = re.compile(
    r'format!\(\s*"(?:[^"\\]|\\.)*?(?:\{' + ERROR_NAME + r'(?::[^}?]*)?\}'
    r'|\{(?::[^}?]*)?\}(?:[^"\\]|\\.)*"\s*,\s*&?' + ERROR_NAME + r'\s*[,)])')
STRINGIFIED_FIELD = re.compile(r"\b\w+\s*:\s*&?\b" + ERROR_NAME + r"\s*\.\s*to_string\(\)")
# A let-else on `Ok(..)`: rustfmt starts the statement on a line of its own.
LET_ELSE_ERROR = re.compile(r"^\s*let\s+Ok\(", re.MULTILINE)
DROPPED_QUIETLY = re.compile(
    r"\.or_else\(\s*(?:move\s*)?\|\s*_\w*\s*\||\bErr\(\s*_\s*\)\s*=>\s*(?:return\s+)?None\b")
# Every other arm that takes an error and names it `_` or `_name` (review 2026-11-02
# TST-121): `Err(_) => { return Err(..) }`, `Err(_ignored) => continue`; the arms
# discard and dropped_quietly count (`Err(_) => Err(`, `Err(_) => None`) are left to them.
ERR_ARM = re.compile(r"\bErr\(\s*(_\w*)\s*\)\s*=>")
COUNTED_ARM = re.compile(r"\s*(?:return\s+)?(?:Err\(|None\b)")
# `.unwrap_or_else(|_| ...)`: an Option's closure takes nothing, so `_` is an error.
DROPPED_CLOSURE = re.compile(r"\.unwrap_or_else\(\s*(?:move\s*)?\|\s*_\w*\s*(?::[^|]*)?\|")
# `let _ = call(...);` throws away what the call answers, an error with it.
LET_UNDERSCORE = re.compile(r"\blet\s+_\s*=\s*[^;]*\)\s*;")
GUARDED_ARM = re.compile(r"\bErr\(\s*_\w*\s*\)\s*if\b")
NAMED_UNDERSCORE = re.compile(
    r"^\s*let\s+_(?!\w*(?:lock|guard|permit|reset|restore)\b)[a-z]\w*\s*(?::[^=]+)?=[^=]",
    re.MULTILINE)
OK_STATEMENT = re.compile(r"\.ok\(\)(?:\s*\.\s*is_(?:some|none)\(\))?\s*;")
# What a statement keeps its value with: a binding, an assignment or a return.
KEPT = re.compile(r"^\s*(?:let|return|break)\b|[^=!<>+\-*/%&|^]=[^=>]")


def bare_ok_statements(text: str) -> int:
    """`.ok();` ending a statement that neither binds nor assigns nor returns it."""
    found = 0
    for match in OK_STATEMENT.finditer(text):
        depth, at = 0, match.start() - 1
        while at >= 0:
            char = text[at]
            if char in ")]":
                depth += 1
            elif char in "([":
                depth -= 1
            elif char in ";{}" and depth == 0:
                break
            at -= 1
        statement, nested = text[at + 1:match.start()], 1
        while nested:
            statement, nested = re.subn(r"\((?!\))[^()]*\)", "()", statement)
        if not KEPT.search(statement):
            found += 1
    return found
FROM_DISCARD = re.compile(r"\bfn\s+from\(\s*_\w*\s*:")
# `.is_err() {}` with nothing but comments in the block, whatever the condition holds
# (a struct literal too, review 2026-11-06 P3).
EMPTY_IS_ERR = re.compile(r"\.is_err\(\)\s*\{\s*(?:(?://[^\n]*\n|/\*.*?\*/)\s*)*\}", re.DOTALL)


def nested_parens(levels: int) -> str:
    """Text with parentheses balanced up to `levels` deep."""
    inner = r"[^()]*"
    for _ in range(levels):
        inner = r"[^()]*(?:\(" + inner + r"\)[^()]*)*"
    return inner


# `let (Ok(..) | Err(..)) =`, either way round, `Ok(Some(()))` nested too.
IRREFUTABLE_LET = re.compile(
    r"\blet\s*\(\s*(?:Ok\(" + nested_parens(3) + r"\)\s*\|\s*Err\(|Err\([^()]*\)\s*\|\s*Ok\()")
# `drop(call())` throws away what the call answers as `let _ =` does; `drop(guard)` ends
# a value's life.
DROPPED_CALL = re.compile(r"\bdrop\(\s*[^()]*\(")
# `.unwrap_or_default()` answers an error with a default; a text check cannot tell an
# Option's, so every one is counted (review 2026-11-06 P3: 167 unseen).
OR_DEFAULT = re.compile(r"\.unwrap_or_default\(\)")
EPRINTLN = re.compile(r"\beprintln!\s*\(")
# A variant of an error type (`...Error::Name(_)`) matched to a bare string.
DROPPED_VARIANT = re.compile(r"\b\w*Error::\w+\(\s*_\s*\)\s*=>\s*\"")
# An inline test module at the top level of a Rust file: `#[cfg(test)]` or
# `#[cfg(all(test, ...))]` before `mod name { ... }`.
TEST_MODULE = re.compile(
    r"^#\[cfg\((?:test|all\(test(?:,[^()\n]*)?\))\)\]\s*\n(?:#\[[^\n]*\]\s*\n)*mod \w+ \{\n.*?^\}",
    re.MULTILINE | re.DOTALL)


def result_error_types(text: str) -> list[str]:
    """The error type of every two-argument `Result<T, E>` (and `Ok::<T, E>`) in `text`."""
    found = []
    for opening in RESULT_OPEN.finditer(text):
        depth, comma, at = 0, None, opening.end()
        while at < len(text):
            char = text[at]
            if text.startswith("->", at):
                at += 2
                continue
            if char in "<([":
                depth += 1
            elif char in ")]":
                depth -= 1
            elif char == ">":
                if depth == 0:
                    break
                depth -= 1
            elif char == "," and depth == 0:
                comma = at
            elif char in ";{}":
                break
            at += 1
        if comma is not None and at < len(text) and text[at] == ">":
            found.append(text[comma + 1:at].strip())
    return found


def content_free(type_text: str) -> bool:
    path = type_text.split("<", 1)[0].strip()
    return any(path == entry or path.endswith("::" + entry) for entry in CONTENT_FREE)


def file_counts(text: str, entry_point: bool = False) -> dict[str, int]:
    text = TEST_MODULE.sub("", text)
    discards = sum(1 for match in DISCARD.finditer(text)
                   if match.group(1) is None or not content_free(match.group(1)))
    discards += len(DISCARD_OTHERWISE.findall(text))
    errors = result_error_types(text)
    return {"discard": discards, "string": len(STRING.findall(text)),
            "unreachable": len(UNREACHABLE.findall(text)),
            "silent_unreachable": len(SILENT_UNREACHABLE.findall(text)),
            "string_result": sum(1 for error in errors if error in ("String", "std::string::String")),
            "str_result": sum(1 for error in errors if STR_ERROR.fullmatch(error)),
            "stringified_cause": len(STRINGIFIED_CAUSE.findall(text)),
            "debug_formatted": len(DEBUG_FORMATTED.findall(text)),
            "stringified_variant": len(STRINGIFIED_VARIANT.findall(text)),
            "formatted_variant": len(FORMATTED_VARIANT.findall(text)),
            "unwrap": len(UNWRAP.findall(text)),
            "dropped_variant": len(DROPPED_VARIANT.findall(text)),
            "dropped_error": len(DROPPED_ERROR.findall(text)),
            "formatted_cause": len(FORMATTED_CAUSE.findall(text)),
            "stringified_field": len(STRINGIFIED_FIELD.findall(text)),
            "let_else_error": len(LET_ELSE_ERROR.findall(text)),
            "dropped_quietly": len(DROPPED_QUIETLY.findall(text)),
            "dropped_arm": sum(1 for arm in ERR_ARM.finditer(text)
                               if arm.group(1) != "_" or not COUNTED_ARM.match(text, arm.end())),
            "dropped_closure": len(DROPPED_CLOSURE.findall(text)),
            "let_underscore": len(LET_UNDERSCORE.findall(text)),
            "guarded_arm": len(GUARDED_ARM.findall(text)),
            "named_underscore": len(NAMED_UNDERSCORE.findall(text)),
            "ok_statement": bare_ok_statements(text),
            "from_discard": len(FROM_DISCARD.findall(text)),
            "empty_is_err": len(EMPTY_IS_ERR.findall(text)),
            "irrefutable_let": len(IRREFUTABLE_LET.findall(text)),
            "dropped_call": len(DROPPED_CALL.findall(text)),
            "or_default": len(OR_DEFAULT.findall(text)),
            "eprintln": window_eprintlns(text) if entry_point else len(EPRINTLN.findall(text))}


WINDOW_LOG = re.compile(r"\blog_window_to\s*\(")


def window_eprintlns(text: str) -> int:
    """`eprintln!` an entry point runs once its window logs to a file: from
    `log_window_to(` to the end of the function calling it (review 2026-11-06
    UI-181). Its command line's branches before that write to the console."""
    count = 0
    for call in WINDOW_LOG.finditer(text):
        depth, at = 0, call.end()
        while at < len(text) and depth >= 0:
            depth += {"{": 1, "}": -1}.get(text[at], 0)
            at += 1
        count += len(EPRINTLN.findall(text, call.end(), at))
    return count


FORMS = tuple(file_counts(""))
FORMS_LINE = "# forms: "


def current_counts(root: Path = ROOT) -> dict[str, int]:
    counts: dict[str, int] = {}
    for path in package_files(root, "src/**/*.rs"):
        relative = path.relative_to(root)
        if EXCLUDED_PARTS & set(relative.parts[:-1]) or path.stem.endswith("tests"):
            continue
        entry_point = path.name == "main.rs" or "bin" in relative.parts[:-1]
        text = path.read_text(encoding="utf-8", errors="replace")
        for form, found in file_counts(text, entry_point).items():
            if found:
                counts[f"{relative.as_posix()} {form}"] = found
    return counts


DROPPED_USE = re.compile(r"\bdropped\(")


def dropped_uses(root: Path = ROOT) -> int:
    """The calls of `dropped(result, Reason::...)` in production source."""
    return sum(len(DROPPED_USE.findall(TEST_MODULE.sub("", path.read_text(
        encoding="utf-8", errors="replace"))))
        for path in package_files(root, "src/**/*.rs"))


def read_baseline(root: Path = ROOT) -> dict[str, int]:
    baseline: dict[str, int] = {}
    path = root / BASELINE_NAME
    if path.exists():
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip() and not line.startswith("#"):
                key, count = line.rsplit(" ", 1)
                baseline[key] = int(count)
    return baseline


def recorded_forms(root: Path = ROOT) -> set[str] | None:
    """The forms the baseline records; `None` without a baseline. A baseline from
    before the `# forms:` line records the forms it has entries of."""
    path = root / BASELINE_NAME
    if not path.exists():
        return None
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith(FORMS_LINE):
            return set(line[len(FORMS_LINE):].split())
    return {key.rsplit(" ", 1)[1] for key in read_baseline(root)}


def check(root: Path = ROOT, update: bool = False) -> int:
    counts = current_counts(root)
    old = read_baseline(root)
    if update:
        recorded = recorded_forms(root)
        grown = {key: value for key, value in counts.items() if value > old.get(key, 0)
                 and (recorded is None or key.rsplit(" ", 1)[1] in recorded)}
        if recorded is not None and grown:
            print("Refusing to grow the baseline:", *sorted(grown), sep="\n  ")
            return 1
        lines = [f"{key} {value}" for key, value in sorted(counts.items())]
        (root / BASELINE_NAME).write_text(
            "# Remaining errors that drop their cause or cases the types still allow"
            " (<file> <form> <count>, forms in scripts/check_error_hygiene.py);"
            " may only shrink.\n"
            + f"{FORMS_LINE}{' '.join(FORMS)}\n"
            + "\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
        print(f"baseline: {sum(counts.values())} occurrences in {len(counts)} entries")
        return 0
    failures = [f"{key}: {value} (allowed {old.get(key, 0)})"
                for key, value in sorted(counts.items()) if value > old.get(key, 0)]
    if failures:
        print("New errors that drop their cause; carry the source error or return a "
              "ketchup_rejection::Rejection (AGENTS.md §2):")
        print(*failures, sep="\n  ")
        return 1
    print(f"{dropped_uses(root)} errors dropped on purpose through ketchup_rejection::dropped")
    shrunk = sum(old.values()) - sum(counts.values())
    if shrunk > 0:
        print(f"OK; {shrunk} occurrences removed since the baseline. "
              "Run with --update to lock in the progress.")
    else:
        print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(check(update="--update" in sys.argv))
