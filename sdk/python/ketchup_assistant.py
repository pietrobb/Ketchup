from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent))

from ketchup_assistant_protocol import (  # noqa: E402
    AssistantSidecarBase,
    INSPECT_DOCUMENT_PARAMETERS,
    LIST_VALIDATORS_PARAMETERS,
    MAX_INSPECT_ROUNDS,
    MAX_LINE_BYTES,
    MAX_MESSAGE_CHARS,
    MAX_U64,
    MEASURE_BOUNDS_PARAMETERS,
    PLAN_LINEAR_ARRAY_PARAMETERS,
    PLAN_PLACEMENT_PARAMETERS,
    PROJECT_MEMORY_SCHEMA,
    RUN_VALIDATORS_PARAMETERS,
    ProtocolError,
    ProviderExchange,
    SYSTEM_PROMPT,
    _anthropic_output_text,
    _anthropic_tool_calls,
    _inspect_document,
    _measure_bounds,
    _openai_tool_calls,
    _parse_assistant_result,
    _plan_linear_array,
    _plan_placement,
    _read_only_tool_result,
    _validate_cad_edit_program,
    _validate_planned_linear_array_answer,
    _validate_planned_placement_answer,
    _validate_selected_parameter_edit_answer,
    _validate_selected_profile_translation_answer,
)

PUBLIC_PROVIDERS = frozenset({"anthropic-api", "openai-api"})


class PublicAssistantSidecar(AssistantSidecarBase):
    distribution = "public-api"
    providers = PUBLIC_PROVIDERS
    provider_rejection = "unsupported public provider"
    distribution_rejection = "public sidecar rejects non-public distributions"


def send_public_exchange(
    provider: str, model: str, message: str, history: tuple[dict, ...]
) -> ProviderExchange:
    if provider == "anthropic-api":
        api_key = os.environ.get("ANTHROPIC_API_KEY")
        if not api_key:
            raise ProtocolError("ANTHROPIC_API_KEY is not set")
        url = "https://api.anthropic.com/v1/messages"
        headers = {"x-api-key": api_key, "anthropic-version": "2023-06-01"}
        tools = [
            {
                "name": "inspect_document",
                "description": "Read the current selection or exact occurrence IDs from the revision-bound Kečup context. This tool cannot mutate anything.",
                "input_schema": INSPECT_DOCUMENT_PARAMETERS,
            },
            {
                "name": "measure_bounds",
                "description": "Measure world-space AABB sizes, centers, and pairwise clearances for the current selection or exact occurrence IDs. This tool cannot mutate anything.",
                "input_schema": MEASURE_BOUNDS_PARAMETERS,
            },
            {
                "name": "plan_placement",
                "description": "Compute the exact translation to place one occurrence on a chosen side of another with a gap and orthogonal alignment. This tool cannot mutate anything.",
                "input_schema": PLAN_PLACEMENT_PARAMETERS,
            },
            {
                "name": "plan_linear_array",
                "description": "Compute the exact step for a touching or gapped linear array of existing occurrences. This tool cannot mutate anything.",
                "input_schema": PLAN_LINEAR_ARRAY_PARAMETERS,
            },
            {
                "name": "list_validators",
                "description": "List every validator Kečup can run on the current document, with what each one checks. This tool cannot mutate anything.",
                "input_schema": LIST_VALIDATORS_PARAMETERS,
            },
            {
                "name": "run_validators",
                "description": "Read the findings of the named validators on the current revision, including the parts each finding refers to and any honest not-evaluated reason. This tool cannot mutate anything.",
                "input_schema": RUN_VALIDATORS_PARAMETERS,
            },
        ]
        messages = [*history, {"role": "user", "content": message}]
        payload = {
            "model": model,
            "max_tokens": 4096,
            "system": SYSTEM_PROMPT,
            "messages": messages,
            "tools": tools,
            "tool_choice": {"type": "auto", "disable_parallel_tool_use": True},
        }
        seen_call_ids = set()
        seen_queries = set()
        planned_placements = []
        planned_arrays = []
        rounds = 0
        total_input_tokens = 0
        total_output_tokens = 0
        total_cache_read_tokens = 0
        total_cache_write_tokens = 0
        started = time.monotonic()
        while True:
            data = _post_json(url, payload, headers)
            usage = data.get("usage", {})
            total_input_tokens += int(usage.get("input_tokens", 0) or 0)
            total_output_tokens += int(usage.get("output_tokens", 0) or 0)
            total_cache_read_tokens += int(usage.get("cache_read_input_tokens", 0) or 0)
            total_cache_write_tokens += int(usage.get("cache_creation_input_tokens", 0) or 0)
            calls = _anthropic_tool_calls(data)
            if not calls:
                answer = _anthropic_output_text(data)
                _validate_planned_placement_answer(answer, planned_placements)
                _validate_planned_linear_array_answer(answer, planned_arrays)
                _validate_selected_profile_translation_answer(answer, message)
                _validate_selected_parameter_edit_answer(answer, message)
                return ProviderExchange(
                    text=answer,
                    model=data.get("model") or model,
                    system_prompt=SYSTEM_PROMPT,
                    request_payload=payload,
                    input_tokens=total_input_tokens,
                    output_tokens=total_output_tokens,
                    cache_read_tokens=total_cache_read_tokens,
                    cache_write_tokens=total_cache_write_tokens,
                    stop_reason=str(data.get("stop_reason") or ""),
                    duration_ms=int((time.monotonic() - started) * 1000),
                )
            if len(calls) != 1:
                raise ProtocolError("provider requested more than one document inspection")
            if rounds >= MAX_INSPECT_ROUNDS:
                raise ProtocolError("provider exceeded the document inspection limit")
            call = calls[0]
            if (
                call.get("name") not in {"inspect_document", "measure_bounds", "plan_placement", "plan_linear_array"}
                or not isinstance(call.get("id"), str)
                or not call["id"]
            ):
                raise ProtocolError("provider requested an unknown or invalid tool")
            result = _read_only_tool_result(message, call.get("name"), call.get("input"))
            fingerprint = (call["name"], tuple(sorted(result["occurrence_ids"])))
            if call["id"] in seen_call_ids:
                raise ProtocolError("provider repeated a document inspection call ID")
            if fingerprint in seen_queries:
                raise ProtocolError("provider repeated a document inspection query")
            seen_call_ids.add(call["id"])
            seen_queries.add(fingerprint)
            if result["tool"] == "plan_placement":
                planned_placements.append(result)
            elif result["tool"] == "plan_linear_array":
                planned_arrays.append(result)
            rounds += 1
            messages = [
                *messages,
                {"role": "assistant", "content": data["content"]},
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "tool_use_id": call["id"],
                            "content": json.dumps(
                                result, ensure_ascii=False, separators=(",", ":"), sort_keys=True
                            ),
                        }
                    ],
                },
            ]
            payload["messages"] = messages
    if provider == "openai-api":
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            raise ProtocolError("OPENAI_API_KEY is not set")
        url = "https://api.openai.com/v1/responses"
        headers = {"Authorization": f"Bearer {api_key}"}
        tools = [
            {
                "type": "function",
                "name": "inspect_document",
                "description": "Read the current selection or exact occurrence IDs from the revision-bound Kečup context. This tool cannot mutate anything.",
                "parameters": INSPECT_DOCUMENT_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "measure_bounds",
                "description": "Measure world-space AABB sizes, centers, and pairwise clearances for the current selection or exact occurrence IDs. This tool cannot mutate anything.",
                "parameters": MEASURE_BOUNDS_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "plan_placement",
                "description": "Compute the exact translation to place one occurrence on a chosen side of another with a gap and orthogonal alignment. This tool cannot mutate anything.",
                "parameters": PLAN_PLACEMENT_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "plan_linear_array",
                "description": "Compute the exact step for a touching or gapped linear array of existing occurrences. This tool cannot mutate anything.",
                "parameters": PLAN_LINEAR_ARRAY_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "list_validators",
                "description": "List every validator Kečup can run on the current document, with what each one checks. This tool cannot mutate anything.",
                "parameters": LIST_VALIDATORS_PARAMETERS,
                "strict": True,
            },
            {
                "type": "function",
                "name": "run_validators",
                "description": "Read the findings of the named validators on the current revision, including the parts each finding refers to and any honest not-evaluated reason. This tool cannot mutate anything.",
                "parameters": RUN_VALIDATORS_PARAMETERS,
                "strict": True,
            },
        ]
        inputs = [*history, {"role": "user", "content": message}]
        payload = {
            "model": model,
            "instructions": SYSTEM_PROMPT,
            "input": inputs,
            "store": False,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": False,
        }
        seen_call_ids = set()
        seen_queries = set()
        planned_placements = []
        planned_arrays = []
        rounds = 0
        total_input_tokens = 0
        total_output_tokens = 0
        total_cache_read_tokens = 0
        started = time.monotonic()
        while True:
            data = _post_json(url, payload, headers)
            usage = data.get("usage", {})
            total_input_tokens += int(usage.get("input_tokens", 0) or 0)
            total_output_tokens += int(usage.get("output_tokens", 0) or 0)
            input_details = usage.get("input_tokens_details", {})
            total_cache_read_tokens += int(input_details.get("cached_tokens", 0) or 0)
            calls = _openai_tool_calls(data)
            if not calls:
                answer = _openai_output_text(data)
                _validate_planned_placement_answer(answer, planned_placements)
                _validate_planned_linear_array_answer(answer, planned_arrays)
                _validate_selected_profile_translation_answer(answer, message)
                _validate_selected_parameter_edit_answer(answer, message)
                return ProviderExchange(
                    text=answer,
                    model=data.get("model") or model,
                    system_prompt=SYSTEM_PROMPT,
                    request_payload=payload,
                    input_tokens=total_input_tokens,
                    output_tokens=total_output_tokens,
                    cache_read_tokens=total_cache_read_tokens,
                    stop_reason=str(data.get("status") or ""),
                    duration_ms=int((time.monotonic() - started) * 1000),
                )
            if len(calls) != 1:
                raise ProtocolError("provider requested more than one document inspection")
            if rounds >= MAX_INSPECT_ROUNDS:
                raise ProtocolError("provider exceeded the document inspection limit")
            call = calls[0]
            if (
                call.get("name") not in {"inspect_document", "measure_bounds", "plan_placement", "plan_linear_array"}
                or not isinstance(call.get("call_id"), str)
                or not call["call_id"]
                or not isinstance(call.get("arguments"), str)
            ):
                raise ProtocolError("provider requested an unknown or invalid tool")
            try:
                arguments = json.loads(call["arguments"])
            except json.JSONDecodeError as error:
                raise ProtocolError("provider tool arguments are invalid JSON") from error
            result = _read_only_tool_result(message, call.get("name"), arguments)
            fingerprint = (call["name"], tuple(sorted(result["occurrence_ids"])))
            if call["call_id"] in seen_call_ids:
                raise ProtocolError("provider repeated a document inspection call ID")
            if fingerprint in seen_queries:
                raise ProtocolError("provider repeated a document inspection query")
            seen_call_ids.add(call["call_id"])
            seen_queries.add(fingerprint)
            if result["tool"] == "plan_placement":
                planned_placements.append(result)
            elif result["tool"] == "plan_linear_array":
                planned_arrays.append(result)
            rounds += 1
            inputs = [
                *inputs,
                *data["output"],
                {
                    "type": "function_call_output",
                    "call_id": call["call_id"],
                    "output": json.dumps(
                        result, ensure_ascii=False, separators=(",", ":"), sort_keys=True
                    ),
                },
            ]
            payload["input"] = inputs
    raise ProtocolError("unsupported public provider")


def send_public_request(
    provider: str, model: str, message: str, history: tuple[dict, ...]
) -> str:
    return send_public_exchange(provider, model, message, history).text


def _post_json(url: str, payload: dict, headers: dict[str, str]) -> dict:
    request = urllib.request.Request(
        url,
        data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json", **headers},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            return json.load(response)
    except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as error:
        raise ProtocolError(f"provider request failed: {type(error).__name__}") from error


def _openai_output_text(data: dict) -> str:
    parts = []
    for item in data.get("output", []):
        if item.get("type") != "message":
            continue
        for content in item.get("content", []):
            if content.get("type") == "output_text":
                parts.append(content.get("text", ""))
    return "".join(parts).strip()


def run() -> int:
    def write_line(text: str) -> None:
        sys.stdout.write(text)
        sys.stdout.flush()

    sidecar = PublicAssistantSidecar(send_public_exchange)
    return sidecar.serve(
        lambda: sys.stdin.buffer.readline(MAX_LINE_BYTES + 1),
        write_line,
    )


if __name__ == "__main__":
    raise SystemExit(run())
