"""The plugin SDK speaks the host's protocol byte for byte (review SDK-1)."""

import io
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "sdk" / "python"))

import ketchup_sdk  # noqa: E402


def test_a_request_is_utf8_with_a_bare_newline_whatever_the_console_code_page():
    child = (
        "from ketchup_sdk import Client\n"
        "Client._send('APPLY\\t{\"name\":\"Kečup — stôl\"}')\n"
    )
    environment = dict(os.environ, PYTHONIOENCODING="cp1252", PYTHONUTF8="0")
    environment["PYTHONPATH"] = str(ROOT / "sdk" / "python")
    finished = subprocess.run(
        [sys.executable, "-c", child], env=environment, capture_output=True, timeout=30
    )
    assert finished.returncode == 0, finished.stderr.decode(errors="replace")
    assert finished.stdout == 'APPLY\t{"name":"Kečup — stôl"}\n'.encode("utf-8")


class _Stdin:
    def __init__(self, data: bytes):
        self.buffer = io.BytesIO(data)


def test_a_response_line_as_long_as_the_host_may_send_is_read(monkeypatch):
    longest = b"S" * ketchup_sdk._MAX_HOST_LINE_BYTES
    monkeypatch.setattr(sys, "stdin", _Stdin(longest + b"\n"))
    assert ketchup_sdk.Client._receive() == longest.decode()

    monkeypatch.setattr(sys, "stdin", _Stdin(longest + b"S\n"))
    with pytest.raises(ketchup_sdk.ProtocolError, match="byte limit"):
        ketchup_sdk.Client._receive()


def test_the_sdk_limits_are_the_hosts():
    plugin = (ROOT / "crates/ketchup-scheduler/src/plugin.rs").read_text(encoding="utf-8")
    assert "MAX_PLUGIN_REQUEST_LINE_BYTES: usize = 4 * 1024;" in plugin
    assert ketchup_sdk._MAX_REQUEST_LINE_BYTES == 4 * 1024
    assert (
        "MAX_PLUGIN_RESPONSE_LINE_BYTES: usize = 2 * PluginLimits::HOST_MAX.max_query_bytes + 32;"
        in plugin
    )
    extension = (ROOT / "crates/ketchup-assistant/src/extension.rs").read_text(encoding="utf-8")
    host_max = re.search(
        r"pub const HOST_MAX: Self = Self \{.*?max_query_bytes: ([^,]+),", extension, re.S
    )
    assert host_max and host_max.group(1).replace("_", "") == "64 * 1024"
    assert ketchup_sdk._HOST_MAX_QUERY_BYTES == 64 * 1024
