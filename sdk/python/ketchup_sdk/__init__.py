from dataclasses import dataclass
import json
import sys

PROTOCOL_V1 = "ketchup.plugin.v1"
QUERY_AGENT_STATE = "query.agent-state.v1"
EDIT_CAD_PROGRAM = "edit.cad-program.v1"
# The host's own limits (ketchup-scheduler MAX_PLUGIN_REQUEST_LINE_BYTES and
# MAX_PLUGIN_RESPONSE_LINE_BYTES): a StateView of HOST_MAX max_query_bytes
# arrives as hex, twice as long, behind its envelope.
_MAX_REQUEST_LINE_BYTES = 4 * 1024
_HOST_MAX_QUERY_BYTES = 64 * 1024
_MAX_HOST_LINE_BYTES = 2 * _HOST_MAX_QUERY_BYTES + 32


class ProtocolError(RuntimeError):
    pass


@dataclass(frozen=True)
class Limits:
    max_requests: int = 4
    max_query_bytes: int = 32 * 1024
    max_operations: int = 1


@dataclass(frozen=True)
class Manifest:
    package: str
    version: str
    principal_id: int
    capabilities: tuple[str, ...]
    limits: Limits = Limits()


class Client:
    def __init__(self, manifest: Manifest):
        self._manifest = manifest
        self._started = False
        self._finished = False

    def start(self) -> None:
        if self._started:
            raise ProtocolError("plugin client was already started")
        limits = self._manifest.limits
        self._send(
            "\t".join(
                (
                    "HELLO",
                    PROTOCOL_V1,
                    self._manifest.package,
                    self._manifest.version,
                    str(self._manifest.principal_id),
                    ",".join(self._manifest.capabilities),
                    str(limits.max_requests),
                    str(limits.max_query_bytes),
                    str(limits.max_operations),
                )
            )
        )
        if self._receive() != f"READY\t{PROTOCOL_V1}":
            raise ProtocolError("host rejected the plugin handshake")
        self._started = True

    def query_agent_state(self) -> str:
        self._require_active()
        self._send("QUERY\tAGENT_STATE")
        fields = self._receive().split("\t")
        if len(fields) != 3 or fields[0] != "STATE":
            raise ProtocolError("host returned a malformed StateView response")
        try:
            expected_bytes = int(fields[1])
            payload = bytes.fromhex(fields[2])
            state = payload.decode("utf-8")
        except (ValueError, UnicodeDecodeError) as error:
            raise ProtocolError("host returned invalid StateView bytes") from error
        if len(payload) != expected_bytes:
            raise ProtocolError("host StateView length did not match its envelope")
        return state

    def inputs(self) -> dict:
        """The typed inputs the host validated against the plugin's manifest."""
        self._require_active()
        self._send("QUERY\tINPUTS")
        kind, _, payload = self._receive().partition("\t")
        if kind != "INPUTS":
            raise ProtocolError("host returned a malformed INPUTS response")
        try:
            inputs = json.loads(payload)
        except ValueError as error:
            raise ProtocolError("host returned invalid INPUTS JSON") from error
        if not isinstance(inputs, dict):
            raise ProtocolError("host INPUTS is not an object")
        return inputs

    def apply_operations(self, operations: list[dict]) -> int:
        """Sends typed CAD operations, written as for MCP `apply_and_verify`;
        returns how many the host accepted. The host commits them as one Undo step."""
        self._require_active()
        self._send("APPLY\t" + json.dumps({"operations": operations}, separators=(",", ":")))
        fields = self._receive().split("\t")
        if len(fields) != 2 or fields[0] != "ACCEPTED":
            raise ProtocolError("host returned a malformed APPLY response")
        try:
            return int(fields[1])
        except ValueError as error:
            raise ProtocolError("host returned an invalid operation count") from error

    def finish(self) -> None:
        self._require_active()
        self._send("DONE")
        if self._receive() != "BYE":
            raise ProtocolError("host did not close the plugin session")
        self._finished = True

    def _require_active(self) -> None:
        if not self._started or self._finished:
            raise ProtocolError("plugin client session is not active")

    @staticmethod
    def _send(line: str) -> None:
        encoded = line.encode("utf-8")
        if len(encoded) > _MAX_REQUEST_LINE_BYTES:
            raise ProtocolError("plugin request exceeds the SDK byte limit")
        # UTF-8 bytes and a bare newline whatever the console code page, as the host reads them.
        sys.stdout.flush()
        sys.stdout.buffer.write(encoded + b"\n")
        sys.stdout.buffer.flush()

    @staticmethod
    def _receive() -> str:
        line = sys.stdin.buffer.readline(_MAX_HOST_LINE_BYTES + 2)
        if not line:
            raise ProtocolError("host closed the protocol")
        content = line.rstrip(b"\r\n")
        if len(content) > _MAX_HOST_LINE_BYTES or (
            len(line) > _MAX_HOST_LINE_BYTES + 1 and not line.endswith(b"\n")
        ):
            raise ProtocolError("host response exceeds the SDK byte limit")
        try:
            return content.decode("utf-8")
        except UnicodeDecodeError as error:
            raise ProtocolError("host response is not UTF-8") from error
