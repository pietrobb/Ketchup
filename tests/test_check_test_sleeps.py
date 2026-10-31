import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("check_test_sleeps", ROOT / "scripts" / "check_test_sleeps.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


PRODUCTION = (
    "fn poll() { std::thread::sleep(POLL); }\n"
    "#[cfg(test)]\n"
    "mod tests {\n"
    "    fn wait() { std::thread::sleep(TICK); }\n"
    "}\n"
)


def test_counts_only_test_code(tmp_path):
    write(tmp_path, "crates/a/src/lib.rs", PRODUCTION)
    write(tmp_path, "crates/a/src/worker_tests.rs", "fn t() { sleep(A); sleep(B); }\n")
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { thread::sleep(A); }\n")
    assert checker.current_counts(tmp_path) == {
        "crates/a/src/lib.rs": 1,
        "crates/a/src/worker_tests.rs": 2,
        "crates/a/tests/flow.rs": 1,
    }


def test_counts_the_viewer_crates_too(tmp_path):
    write(tmp_path, "viewer/android/src/lib.rs", PRODUCTION)
    write(tmp_path, "viewer/web/tests/smoke.rs", "fn t() { sleep(A); }\n")
    assert checker.current_counts(tmp_path) == {
        "viewer/android/src/lib.rs": 1,
        "viewer/web/tests/smoke.rs": 1,
    }


def test_counts_spaced_and_park_timeout_sleeps(tmp_path):
    # Review 2026-10-19 P3: `sleep (` and `park_timeout` waited for a fixed time unseen.
    write(
        tmp_path,
        "crates/a/tests/flow.rs",
        "fn t() {\n"
        "    std::thread::sleep (TICK);\n"
        "    std::thread::park_timeout(TICK);\n"
        "    thread::park_timeout_ms(5);\n"
        "    std::thread::sleep_ms(5);\n"
        "    let asleep = sleeper(1);\n"
        "}\n",
    )
    assert checker.current_counts(tmp_path) == {"crates/a/tests/flow.rs": 4}


def test_counts_the_sleep_calls_of_python_tests(tmp_path):
    # Review 2026-10-21 P3: Python tests were not scanned. A sleep named in a string is not
    # a call, and a script outside the tests is not counted.
    write(tmp_path, "tests/test_flow.py", "import time\ntime.sleep(0.1)\nFIXTURE = 'sleep(A)'\n")
    write(tmp_path, "sdk/python/tests/test_client.py", "from time import sleep\nsleep(1)\nsleep(2)\n")
    write(tmp_path, "scripts/tool.py", "import time\ntime.sleep(1)\n")
    assert checker.current_counts(tmp_path) == {
        "sdk/python/tests/test_client.py": 2,
        "tests/test_flow.py": 1,
    }


def test_counts_sleeps_called_by_another_name(tmp_path):
    # Review 2026-10-24 (§3.7): `use thread::sleep as nap`, `from time import sleep as
    # pause` and a fresh `Event().wait(0.5)`, which nothing can set, slept unseen.
    write(
        tmp_path,
        "crates/a/tests/flow.rs",
        "use std::thread::sleep as nap;\n"
        "use std::thread::{park_timeout as doze, spawn};\n"
        "fn t() { nap(TICK); doze (TICK); napkin(1); }\n",
    )
    write(
        tmp_path,
        "tests/test_flow.py",
        "import threading\n"
        "from time import sleep as pause\n"
        "from threading import Event\n"
        "pause(0.1)\n"
        "Event().wait(0.5)\n"
        "threading.Event().wait(timeout=1)\n"
        "done = Event()\n"
        "done.wait(5)\n",
    )
    assert checker.current_counts(tmp_path) == {
        "crates/a/tests/flow.rs": 2,
        "tests/test_flow.py": 3,
    }


def test_a_sleep_in_a_helper_counts_at_every_call_of_the_helper(tmp_path):
    # Review 2026-10-29 TST-42: three polling loops moved into one `ask_until` dropped
    # the count from 3 to 2 while the tests slept as before. A helper that calls a
    # sleeping helper sleeps too; one that nothing calls still counts its own sleeps.
    write(
        tmp_path,
        "tests/test_flow.py",
        "import time\n"
        "def ask_until(ask, every):\n"
        "    while not ask():\n"
        "        time.sleep(every)\n"
        "def settled(reply):\n"
        "    return ask_until(lambda: reply, 1)\n"
        "def unused():\n"
        "    time.sleep(1)\n"
        "def test_a():\n"
        "    settled(1)\n"
        "    settled(2)\n"
        "    ask_until(ready, 1)\n"
        "    client.ask_until(ready)\n",
    )
    write(
        tmp_path,
        "crates/a/tests/flow.rs",
        "fn call(x: u8) -> u8 {\n    loop {\n        std::thread::sleep(TICK);\n    }\n}\n"
        "fn twice() {\n    call(1);\n    call(2);\n}\n"
        "fn two_naps() {\n    sleep(A);\n    sleep(B);\n}\n"
        "#[test]\nfn t() {\n    call(1);\n    twice();\n    twice();\n    two_naps();\n"
        "    other.call(3);\n    // call(4)\n}\n",
    )
    # A call counts the sleeps the helper makes: call 1, twice 2 (each), two_naps 2.
    assert checker.current_counts(tmp_path) == {
        "crates/a/tests/flow.rs": 7,
        "tests/test_flow.py": 4,
    }


def test_growth_fails_and_shrinking_passes(tmp_path):
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { sleep(A); sleep(B); }\n")
    assert checker.check(tmp_path, update=True) == 0
    assert checker.check(tmp_path) == 0
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { sleep(A); sleep(B); sleep(C); }\n")
    assert checker.check(tmp_path) == 1
    assert checker.check(tmp_path, update=True) == 1
    write(tmp_path, "crates/a/tests/flow.rs", "fn t() { sleep(A); }\n")
    assert checker.check(tmp_path) == 0
    write(tmp_path, "crates/a/tests/new.rs", "fn t() { sleep(A); }\n")
    assert checker.check(tmp_path) == 1


def test_the_repository_is_within_its_baseline():
    assert checker.check(ROOT) == 0
