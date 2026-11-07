import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_error_hygiene", ROOT / "scripts" / "check_error_hygiene.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_counts_discarded_causes_and_bare_text_errors_in_production_source(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'let a = x.map_err(|_| "bad")?;\n'
        "let b = y.map_err(|_error| Error::Io)?;\n"
        "let c = z.map_err(move |_| Error::Io)?;\n"
        'return Err("no such part".to_owned());\n'
        'return Err( "a \\"quoted\\" name" .into() );\n'
        'Err("x".to_string())\n'
        "let kept = w.map_err(|error| Error::Io(error))?;\n"
        'let formatted = Err(format!("{x}"));\n',
    )
    write(tmp_path, "crates/a/src/scene_tests.rs", 'x.map_err(|_| "bad")?;\n')
    write(tmp_path, "crates/a/tests/it.rs", 'x.map_err(|_| "bad")?;\n')
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs discard": 3,
        "crates/a/src/lib.rs string": 3,
    }


def test_counts_unreachable_cases_in_production_source_only(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'Kind::Circle => unreachable!("handled above"),\n'
        "_ => unreachable! ()\n"
        "// not_unreachable!(x) and the word unreachable alone do not count\n"
        '#[cfg(test)]\nmod tests {\n    fn g() { unreachable!() }\n}\n',
    )
    write(tmp_path, "crates/a/tests/it.rs", "unreachable!();\n")
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs unreachable": 2,
        "crates/a/src/lib.rs silent_unreachable": 1,
    }


def test_only_named_content_free_error_types_may_be_dropped(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "let a = u32::try_from(n).map_err(|_: std::num::TryFromIntError| Error::Range)?;\n"
        "let b = lock.map_err(|_: PoisonError<Guard<'_>>| Error::Poisoned)?;\n"
        "let c = s.parse::<u32>().map_err(|_: std::num::ParseIntError| Error::Number)?;\n"
        "let d = write!(out, \"x\").map_err(|_: std::fmt::Error| Error::Limit)?;\n"
        "let e = file.read(b).map_err(|_: std::io::Error| Error::Io)?;\n"
        "let f = key.verify(m, s).map_err(|_: ed25519_dalek::SignatureError| Error::Forged)?;\n"
        "let g = other.check().map_err(|_: SignatureError| Error::Forged)?;\n"
        "let h = path.strip_prefix(root).map_err(|_: std::path::StripPrefixError| Error::Outside)?;\n",
    )
    # Only the io::Error (e) and the unqualified SignatureError (g) drop content.
    assert checker.current_counts(tmp_path) == {"crates/a/src/lib.rs discard": 2}


def test_inline_test_modules_are_not_production_code(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'fn f() -> Result<(), String> { Err("x".to_owned()) }\n'
        '#[cfg(test)]\nmod tests {\n    fn g() { x.map_err(|_| "bad"); }\n}\n'
        '#[cfg(all(test, windows))]\nmod windows_tests {\n    fn h() { y.unwrap(); }\n}\n'
        '#[cfg(not(test))]\nmod production {\n    fn k() { z.unwrap(); }\n}\n',
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs string": 1,
        "crates/a/src/lib.rs string_result": 1,
        "crates/a/src/lib.rs unwrap": 1,
    }


def test_counts_text_errors_formatted_variants_and_unwraps(tmp_path):
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "fn a() -> Result<Vec<Option<u8>>, String> { todo!() }\n"
        "fn b() -> std::result::Result<HashMap<K, V>, String> { todo!() }\n"
        "fn c() -> Result<(), Error> { todo!() }\n"
        "let j = run(|| Ok::<_, String>(1)); let k = Ok::<_, Error>(1);\n"
        "let d = x.map_err(|error| error.to_string())?;\n"
        "let e = x.map_err(move |e| e.to_string())?;\n"
        "let f = x.map_err(|error| Error::Io(error.to_string()))?;\n"
        "Err(e) => Failure::Worker(e.to_string()), Some(error.to_string()), Label(name.to_string())\n"
        'Err(e) => Error::new("invalid_json", e.to_string()), Error::invalid(&error.to_string())\n'
        'Error::new("x", name.to_string()); list.push(error.to_string())\n'
        'Error::Dependency(format!("{a} needs {b}"))\n'
        'Unsupported( format!("x") )\n'
        'Some(format!("x")); Ok(format!("x")); Err(format!("x")); name(format!("x"))\n'
        "let g = h.unwrap();\nlet i = h.unwrap_or(0);\n"
        '_ => unreachable!(),\n_ => unreachable!("said why"),\n',
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs string_result": 3,
        "crates/a/src/lib.rs stringified_cause": 2,
        "crates/a/src/lib.rs stringified_variant": 4,
        "crates/a/src/lib.rs formatted_variant": 2,
        "crates/a/src/lib.rs unwrap": 1,
        "crates/a/src/lib.rs unreachable": 2,
        "crates/a/src/lib.rs silent_unreachable": 1,
    }


def test_a_str_error_type_and_a_tuple_ok_type_count(tmp_path):
    # Review 2026-10-29 TST-43: `Result<_, &'static str>` rose from 166 to 179 unseen, and
    # a tuple Ok type hid a `String` error from string_result.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "fn a() -> Result<(u8, u8), &'static str> { todo!() }\n"
        "fn b() -> Result<Vec<Option<Box<[u8]>>>, &'static str> { todo!() }\n"
        "fn c<'a>(x: &'a str) -> Result<u8, &'a str> { todo!() }\n"
        "let d = Ok::<_, &'static str>(1);\n"
        "fn e() -> Result<(u8, Vec<u8>), String> { todo!() }\n"
        "fn f() -> Result<Box<dyn Fn(u8) -> u8>, String> { todo!() }\n"
        "// Not an error type: none count.\n"
        "fn g() -> Result<&'static str, Error> { todo!() }\n"
        "fn h() -> io::Result<&'static str> { todo!() }\n"
        "fn i() -> HashMap<u8, &'static str> { todo!() }\n",
    )
    # Review 2026-10-26 TST-43: the cause dropped without map_err.
    write(
        tmp_path,
        "crates/b/src/lib.rs",
        "let a = parse(x).ok().ok_or(Error::Bad)?;\n"
        "let b = parse(x).ok() .ok_or_else(|| Error::Bad)?;\n"
        "let c = parse(x).or(Err(Error::Bad))?;\n"
        "match parse(x) { Ok(v) => v, Err(_) => return Err(Error::Bad) }\n"
        "match parse(x) { Ok(v) => Ok(v), Err( _ ) => Err(Error::Bad) }\n"
        "// Keeping the cause, or an Option, does not count.\n"
        "let d = find(x).ok_or(Error::Missing)?; let e = parse(x).or_else(|error| retry(error));\n"
        "match parse(x) { Err(error) => Err(Error::Bad(error)), Ok(v) => Ok(v) }\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs str_result": 4,
        "crates/a/src/lib.rs string_result": 2,
        "crates/b/src/lib.rs discard": 5,
    }


def test_viewer_apps_are_scanned_and_text_or_debug_causes_count(tmp_path):
    write(
        tmp_path,
        "viewer/android/src/files.rs",
        "let a = call().map_err(ToString::to_string)?;\n"
        "let b = call().map_err(std::string::ToString::to_string)?;\n"
        "let c = call().map_err(|error| error.to_string())?;\n",
    )
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'let d = format!("{error:?}");\n'
        'let e = format!("query rejected: {e:#?} at {x}");\n'
        'let f = format!("{:?}", error);\n'
        'let g = format!("{:?}", &io_error);\n'
        '// Debug text of values that are not errors does not count.\n'
        'let h = format!("{point:?}"); let i = format!("{:?}", kind);\n',
    )
    assert checker.current_counts(tmp_path) == {
        "viewer/android/src/files.rs stringified_cause": 3,
        "crates/a/src/lib.rs debug_formatted": 4,
    }


def test_an_error_variant_matched_to_a_bare_code_counts(tmp_path):
    # Review 2026-10-17 MCP-2: `Variant(_) => "code"` dropped the cause unseen.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        'ProjectDrawingsError::Drawing(_) => "drawings_unavailable",\n'
        'SessionError::Commit( _ ) =>"commit_rejected",\n'
        '// Not an error type, a named cause, or no text answer: none count.\n'
        'SpaceRow::Space(_) => "spaces-field-purpose",\n'
        'Self::Geometry(_) => "geometry",\n'
        'DrawError::Write(error) => failed_because("write", error),\n'
        "DrawError::Empty(_) => Error::Empty,\n",
    )
    assert checker.current_counts(tmp_path) == {"crates/a/src/lib.rs dropped_variant": 2}


def test_the_ways_around_the_discard_and_text_forms_count(tmp_path):
    # Review 2026-10-31 TST-80: each of these dropped or flattened a cause unseen.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "let a = call().map_err(drop)?;\n"
        "if let Err(_) = call() { return None; }\n"
        "let b = call().ok()?;\n"
        'let c = format!("{e}");\n'
        'let d = format!("open failed: {error}");\n'
        'let e = format!("{}", err);\n'
        "let f = Error::Io { message: e.to_string() };\n"
        "let g = Failure { path, reason: &source.to_string() };\n"
        "// None of these drop or flatten a cause.\n"
        "let h = call().map(drop);\n"
        'let i = format!("{name}"); let j = format!("{error:?}");\n'
        "let k: String = error.to_string();\n"
        "if let Err(error) = call() { log(error); }\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs dropped_error": 3,
        "crates/a/src/lib.rs formatted_cause": 3,
        "crates/a/src/lib.rs stringified_field": 2,
        "crates/a/src/lib.rs debug_formatted": 1,
    }


def test_an_error_the_else_branch_cannot_see_counts(tmp_path):
    # Review 2026-11-01 TST-100: `let Ok(x) = f() else { .. }` threw away the error of
    # `transport::start` and refused the attach without a reason; `.or_else(|_| Err(..))`
    # and `Err(_) => None` drop it as `map_err(|_| ..)` does.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "let Ok(bridge) = start(context) else {\n    return reject();\n};\n"
        "let Ok(Some(value)) = read(path)\nelse { return; };\n"
        "let b = call().or_else(|_| Err(Refused));\n"
        "let c = match call() { Ok(v) => Some(v), Err(_) => None };\n"
        "// A text check cannot tell a content-free error: this one counts too.\n"
        "let Ok(guard) = lock.lock() else { return; };\n"
        "// None of these drop a cause.\n"
        "let Some(part) = parts.first() else { return; };\n"
        "let d = call().or_else(|error| retry(error));\n"
        "let e = match call() { Ok(v) => v, Err(error) => return Err(error.into()) };\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs let_else_error": 3,
        "crates/a/src/lib.rs dropped_quietly": 2,
    }


def test_the_arms_closures_and_lets_that_drop_an_error_count(tmp_path):
    # Review 2026-11-02 TST-121: none of these was seen.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "let a = match parse(b) { Ok(v) => v, Err(_) => {\n    return Err(Invalid);\n} };\n"
        "let b = match call() { Ok(v) => v, Err(_ignored) => continue };\n"
        "let c = match call() { Ok(v) => v, Err(_named) => None };\n"
        'let d = parse(e).unwrap_or_else(|_| "0".to_owned());\n'
        "let _ = sender.send(message);\n"
        "// Counted by the forms before them, or dropping no error.\n"
        "let f = match call() { Ok(v) => v, Err(_) => return Err(Invalid) };\n"
        "let g = match call() { Ok(v) => Some(v), Err(_) => None };\n"
        "let h = value.unwrap_or_else(|| fallback());\n"
        "let _ = &mut borrowed;\n"
        "let _guard = lock(span);\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs dropped_arm": 3,
        "crates/a/src/lib.rs dropped_closure": 1,
        "crates/a/src/lib.rs let_underscore": 1,
        "crates/a/src/lib.rs discard": 1,
        "crates/a/src/lib.rs dropped_quietly": 1,
    }


def test_the_drops_clippy_cannot_see_count(tmp_path):
    # Review 2026-11-04 TST-140: a guarded arm, a named underscore binding, a bare
    # `.ok();` and a `From` that names its source `_` all slipped past the ratchet.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "match remove(path) { Ok(()) => {}, Err(_) if !path.exists() => {}, Err(e) => return Err(e) }\n"
        "match open(path) { Err(_ignored) if retry => continue, other => other? }\n"
        "let _checksum = manifest.take(32)?;\n"
        "export(&path)\n    .ok();\n"
        "impl From<io::Error> for Error { fn from(_: io::Error) -> Self { Error::Io } }\n"
        "// Guards held to the end of the scope drop no error.\n"
        "let _lock = lock_saves(path)?;\n"
        "let _guard = span.enter();\n"
        "let _permit = permit;\n"
        "let _reset = Reset(FLAG.replace(true));\n"
        "let kept = export(&path).ok();\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs guarded_arm": 2,
        "crates/a/src/lib.rs named_underscore": 1,
        "crates/a/src/lib.rs ok_statement": 1,
        "crates/a/src/lib.rs from_discard": 1,
    }


def test_the_idioms_that_act_as_let_underscore_count(tmp_path):
    # Review 2026-11-04 r19 TST-160: an empty `if ...is_err() {}` with only a
    # comment, an irrefutable `let (Ok(..) | Err(..)) =` and an `eprintln!` a
    # window started without a console throws away slipped past the ratchet;
    # `ketchup_rejection::dropped(result, Reason::...)` is the one way left.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "if child.kill().is_err() {\n    // It has exited already.\n}\n"
        "if ready && send.send(x).is_err() { /* Gone. */ }\n"
        "if remove(&a).is_err() {}\n"
        "let (Ok(()) | Err(SendError(_))) = sender.send(value);\n"
        "let (Ok(())\n    | Err(Failure::Cancelled | Failure::Failed)) =\n    save(&path);\n"
        'eprintln!("import refused: {error}");\n'
        "// What still answers the error is not counted.\n"
        "if send(x).is_err() {\n    return Err(Gone);\n}\n"
        "let (a, b) = pair;\n"
        "dropped(child.kill(), Reason::AlreadyEnded);\n",
    )
    # A command-line entry point writes to its console.
    write(tmp_path, "crates/a/src/main.rs", 'eprintln!("usage: a <file>");\n')
    write(tmp_path, "crates/a/src/bin/tool.rs", 'eprintln!("usage: tool <file>");\n')
    # Review 2026-11-06 UI-181: once its window logs to a file, an entry
    # point has no console either; its command line's branches still do.
    write(
        tmp_path,
        "crates/b/src/main.rs",
        "fn main() {\n"
        '    if verify { eprintln!("usage: b --verify <file>"); return; }\n'
        '    log_window_to("B", "b.log");\n'
        '    if let Err(error) = run(|| { eprintln!("{error}"); }) {\n'
        '        eprintln!("window failed: {error}");\n'
        "    }\n"
        "}\n"
        'fn usage() { eprintln!("usage: b <file>"); }\n',
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs empty_is_err": 3,
        "crates/a/src/lib.rs irrefutable_let": 2,
        "crates/a/src/lib.rs eprintln": 1,
        "crates/b/src/main.rs eprintln": 2,
    }


def test_the_drops_review_1106_found_past_the_ratchet_count(tmp_path):
    # Review 2026-11-06 P3: a struct literal in an empty `is_err` condition,
    # `drop(call())`, `.ok().is_some();`, `Ok(Some(()))` in an irrefutable let and
    # `unwrap_or_default()` all slipped past.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "if tx.send(Msg { a: 1 }).is_err() {}\n"
        "drop(call());\n"
        "drop(sender.send(Msg { a: 1 }));\n"
        "save(&path).ok().is_some();\n"
        "let (Ok(Some(())) | Err(_)) = receiver.recv();\n"
        "let text = read(&path).unwrap_or_default();\n"
        "// What keeps or answers it is not counted.\n"
        "drop(guard);\n"
        "let sent = save(&path).ok().is_some();\n"
        "if tx.send(Msg { a: 1 }).is_err() { return Err(Gone); }\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs empty_is_err": 1,
        "crates/a/src/lib.rs dropped_call": 2,
        "crates/a/src/lib.rs ok_statement": 1,
        "crates/a/src/lib.rs irrefutable_let": 1,
        "crates/a/src/lib.rs or_default": 1,
    }


def test_the_drops_review_1106_r23_found_past_the_ratchet_count(tmp_path):
    # Review 2026-11-06 r23 P3: `.unwrap_or(())` (live in ketchup-viewer/src/gpu.rs),
    # `.err();`, `std::mem::forget(call())`, `unwrap_or_else(Default::default)` and
    # `map_or((), ...)` slipped past; an `eprintln!` in a helper `main.rs` calls after
    # its window logs to a file did too (`bootstrap_failed`).
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "sender.send(result).unwrap_or(());\n"
        "remove(&path).err();\n"
        "std::mem::forget(child.kill());\n"
        "mem::forget(call());\n"
        "let text = read(&path).unwrap_or_else(Default::default);\n"
        "sent.map_or((), |_| ());\n"
        "// What keeps or answers it is not counted.\n"
        "self.failure = report.err();\n"
        "let error = save(&path).err();\n"
        "let size = len.unwrap_or(0);\n"
        "found.remove(oldest);\n",
    )
    write(
        tmp_path,
        "crates/b/src/main.rs",
        "fn main() {\n"
        '    if verify { usage(); return; }\n'
        '    log_window_to("B", "b.log");\n'
        "    if let Err(error) = start() { bootstrap_failed(&error); }\n"
        "}\n"
        'fn usage() { eprintln!("usage: b <file>"); }\n'
        'fn bootstrap_failed(error: &Error) { eprintln!("{error}"); report(error); }\n'
        'fn report(error: &Error) { eprintln!("again: {error}"); bootstrap_failed(error); }\n',
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs or_unit": 2,
        "crates/a/src/lib.rs ok_statement": 1,
        "crates/a/src/lib.rs dropped_call": 2,
        "crates/a/src/lib.rs or_default": 1,
        "crates/b/src/main.rs eprintln": 2,
    }


def test_the_forms_with_a_path_or_a_name_review_1107_found_count(tmp_path):
    # Review 2026-11-07 P3: a default named by its type, `unwrap_or_else(drop)`,
    # `map_or_else(|_| (), ...)`, a bare `.is_ok();`, an empty `if let Ok(()) = ... {}`
    # and `.expect(` slipped past.
    write(
        tmp_path,
        "crates/a/src/lib.rs",
        "let text = read(&path).unwrap_or_else(String::default);\n"
        "let list = read(&path).unwrap_or_else(<_>::default);\n"
        "let bytes = read(&path).unwrap_or_else(std::vec::Vec::default);\n"
        "save(&path).unwrap_or_else(drop);\n"
        "save(&path).map_or_else(|_| (), |()| done());\n"
        "save(&path).is_ok();\n"
        "if let Ok(()) = save(&path) {}\n"
        "if let Ok(_) = tx.send(Msg { a: 1 }) { /* Gone. */ }\n"
        'let value = parse(text).expect("checked above");\n'
        "// What keeps or answers it is not counted.\n"
        "let saved = save(&path).is_ok();\n"
        "if let Ok(()) = save(&path) { done(); }\n"
        "let size = len.unwrap_or_else(|| 0);\n"
        "let kept = found.map_or_else(|| 0, |n| n);\n",
    )
    # In an entry point: `log_window_to` inside an `if`, braces in strings and
    # characters, a helper passed by its name and a method called on `self`.
    write(
        tmp_path,
        "crates/b/src/main.rs",
        "fn main() {\n"
        "    if help { usage(); return; }\n"
        "    if window {\n"
        '        log_window_to("B", "b.log");\n'
        "    }\n"
        '    let close = "}";\n'
        "    let brace = '}';\n"
        '    eprintln!("after the if");\n'
        "    load().unwrap_or_else(bootstrap_failed);\n"
        "    App::start(&app);\n"
        "}\n"
        'fn usage() { eprintln!("usage: b <file>"); }\n'
        'fn bootstrap_failed(error: Error) { eprintln!("{error}"); }\n'
        "impl App {\n"
        "    fn start(&self) { self.fail(); }\n"
        '    fn fail(&self) { eprintln!("failed"); }\n'
        "}\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs or_default": 3,
        "crates/a/src/lib.rs dropped_error": 1,
        "crates/a/src/lib.rs dropped_closure": 1,
        "crates/a/src/lib.rs ok_statement": 1,
        "crates/a/src/lib.rs empty_is_err": 2,
        "crates/a/src/lib.rs expect": 1,
        "crates/b/src/main.rs eprintln": 3,
    }


def test_a_new_form_is_recorded_once_and_then_may_only_shrink(tmp_path):
    # A form the baseline does not list yet is recorded as found when it is
    # added to the check; a form it lists may never grow, even from zero.
    (tmp_path / "scripts").mkdir()
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\n")
    assert checker.check(tmp_path, update=True) == 0
    baseline = tmp_path / checker.BASELINE_NAME
    text = baseline.read_text(encoding="utf-8")
    assert f"# forms: {' '.join(checker.FORMS)}\n" in text
    baseline.write_text(text.replace(" dropped_error", ""), encoding="utf-8")
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\nlet b = c.ok()?;\n")
    assert checker.check(tmp_path) == 1
    assert checker.check(tmp_path, update=True) == 0
    assert checker.read_baseline(tmp_path)["crates/a/src/lib.rs dropped_error"] == 1
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\nlet b = c.ok()?;\nd.ok()?;\n")
    assert checker.check(tmp_path, update=True) == 1
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\nlet b = c.ok()?;\nunreachable!(\"x\");\n")
    assert checker.check(tmp_path, update=True) == 1


def test_the_baseline_has_no_way_to_grow(tmp_path):
    (tmp_path / "scripts").mkdir()
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\n")
    assert checker.check(tmp_path, update=True) == 0
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\nb.map_err(|_| 2);\n")
    assert checker.check(tmp_path, update=True) == 1
    assert "allow_growth" not in checker.check.__code__.co_varnames
    source = (ROOT / "scripts" / "check_error_hygiene.py").read_text(encoding="utf-8")
    assert "--allow-growth" not in source


def test_the_checks_and_baselines_have_an_owner():
    owners = (ROOT / ".github" / "CODEOWNERS").read_text(encoding="utf-8").split()
    assert "/scripts/check_*.py" in owners
    assert "/scripts/*baseline*" in owners


def test_ratchet_refuses_growth_and_locks_in_shrinking(tmp_path, capsys):
    (tmp_path / "scripts").mkdir()
    write(tmp_path, "crates/a/src/lib.rs", 'a.map_err(|_| 1);\nb.map_err(|_| 2);\n')
    assert checker.check(tmp_path, update=True) == 0
    assert checker.check(tmp_path) == 0

    write(tmp_path, "crates/b/src/lib.rs", 'return Err("bad".into());\n')
    assert checker.check(tmp_path) == 1
    assert "crates/b/src/lib.rs string: 1 (allowed 0)" in capsys.readouterr().out
    assert checker.check(tmp_path, update=True) == 1

    (tmp_path / "crates/b/src/lib.rs").unlink()
    write(tmp_path, "crates/a/src/lib.rs", "a.map_err(|_| 1);\n")
    assert checker.check(tmp_path, update=True) == 0
    assert checker.read_baseline(tmp_path) == {"crates/a/src/lib.rs discard": 1}
