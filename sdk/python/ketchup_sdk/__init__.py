from dataclasses import dataclass
import sys

PROTOCOL_V1 = "ketchup.plugin.v1"
QUERY_AGENT_STATE = "query.agent-state.v1"
SET_RULE_DIMENSION = "intent.set-rule-dimension.v1"
SET_FEATURE_DIMENSION = "intent.set-feature-dimension.v1"
_MAX_HOST_LINE_BYTES = 128 * 1024


class ProtocolError(RuntimeError):
    pass


@dataclass(frozen=True)
class Limits:
    max_requests: int = 4
    max_query_bytes: int = 32 * 1024
    max_commands: int = 1
    max_reads: int = 64
    max_writes: int = 1


@dataclass(frozen=True)
class Manifest:
    package: str
    version: str
    principal_id: int
    capabilities: tuple[str, ...]
    limits: Limits = Limits()


@dataclass(frozen=True)
class ProposalReceipt:
    command_digest: str
    result_digest: str
    commands: int
    reads: int
    writes: int


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
                    str(limits.max_commands),
                    str(limits.max_reads),
                    str(limits.max_writes),
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

    def set_rule_dimension(self, target: int, value: str) -> ProposalReceipt:
        return self._propose("SET_RULE_DIMENSION", target, value)

    def set_feature_dimension(self, target: int, value: str) -> ProposalReceipt:
        return self._propose("SET_FEATURE_DIMENSION", target, value)

    def finish(self) -> None:
        self._require_active()
        self._send("DONE")
        if self._receive() != "BYE":
            raise ProtocolError("host did not close the plugin session")
        self._finished = True

    def _propose(self, operation: str, target: int, value: str) -> ProposalReceipt:
        self._require_active()
        if "\t" in value or "\n" in value or "\r" in value:
            raise ValueError("dimension value cannot contain protocol delimiters")
        self._send(f"INTENT\t{operation}\t{target}\t{value}")
        fields = self._receive().split("\t")
        if len(fields) != 6 or fields[0] != "PROPOSAL":
            raise ProtocolError("host returned a malformed Proposal response")
        try:
            return ProposalReceipt(
                command_digest=fields[1],
                result_digest=fields[2],
                commands=int(fields[3]),
                reads=int(fields[4]),
                writes=int(fields[5]),
            )
        except ValueError as error:
            raise ProtocolError("host returned invalid Proposal costs") from error

    def _require_active(self) -> None:
        if not self._started or self._finished:
            raise ProtocolError("plugin client session is not active")

    @staticmethod
    def _send(line: str) -> None:
        encoded = line.encode("utf-8")
        if len(encoded) > 4 * 1024:
            raise ProtocolError("plugin request exceeds the SDK byte limit")
        sys.stdout.write(line + "\n")
        sys.stdout.flush()

    @staticmethod
    def _receive() -> str:
        line = sys.stdin.buffer.readline(_MAX_HOST_LINE_BYTES + 1)
        if not line:
            raise ProtocolError("host closed the protocol")
        if len(line) > _MAX_HOST_LINE_BYTES:
            raise ProtocolError("host response exceeds the SDK byte limit")
        try:
            return line.rstrip(b"\r\n").decode("utf-8")
        except UnicodeDecodeError as error:
            raise ProtocolError("host response is not UTF-8") from error
