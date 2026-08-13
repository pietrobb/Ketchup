import importlib.util
import json
from pathlib import Path
import sys

import pytest

MODULE_PATH = Path(__file__).parents[1] / "sdk" / "python" / "ketchup_assistant.py"
SPEC = importlib.util.spec_from_file_location("ketchup_assistant", MODULE_PATH)
assistant = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = assistant
SPEC.loader.exec_module(assistant)


def hello(provider="anthropic-api", **extra):
    return {
        "type": "hello",
        "protocol_version": 2,
        "distribution": "public-api",
        "provider": provider,
        "model": "claude-sonnet-4-6" if provider == "anthropic-api" else "gpt-5.2",
        "capabilities": ["chat", "local_memory", "query_document", "propose_workflow_intent"],
        **extra,
    }


def test_public_sidecar_conversation_uses_bounded_context_and_no_tools():
    calls = []

    def sender(provider, model, message, history):
        calls.append((provider, model, message, history))
        return json.dumps({"message": "I can prepare a proposal.", "model_intent": None})

    sidecar = assistant.PublicAssistantSidecar(sender)
    ready = sidecar.handle(hello())
    result = sidecar.handle(
        {"type": "chat", "request_id": "r1", "message": "Raise it.", "context": {"selection": [7]}}
    )

    assert ready["distribution"] == "public-api"
    assert ready["model"] == "claude-sonnet-4-6"
    assert result == {
        "type": "chat-result",
        "request_id": "r1",
        "message": "I can prepare a proposal.",
        "model_intent": None,
    }
    assert calls == [
        (
            "anthropic-api",
            "claude-sonnet-4-6",
            '<document-context>{"selection": [7]}</document-context>\n\nRaise it.',
            (),
        )
    ]


def test_public_sidecar_parses_bounded_model_intent_and_rejects_invalid_geometry():
    valid = json.dumps(
        {
            "message": "Prepared one body.",
            "model_intent": {
                "replace_scene": True,
                "boxes": [
                    {
                        "name": "Body",
                        "size_mm": [100, 80, 60],
                        "origin_mm": [10, 20, 30],
                        "subtract_boxes": [
                            {"size_mm": [20, 80, 10], "origin_mm": [30, 0, 50]}
                        ],
                    }
                ],
            },
        }
    )
    result = assistant._parse_assistant_result(valid)
    assert result["model_intent"]["boxes"][0]["origin_mm"] == [10, 20, 30]
    assert result["model_intent"]["boxes"][0]["subtract_boxes"][0]["size_mm"] == [20, 80, 10]

    move = assistant._parse_assistant_result(
        json.dumps(
            {
                "message": "Moved the existing body by 100 mm.",
                "model_intent": {
                    "replace_scene": False,
                    "boxes": [],
                    "translations": [{"occurrence_id": 7, "delta_mm": [100, 0, 0]}],
                },
            }
        )
    )
    assert move["model_intent"]["translations"] == [
        {"occurrence_id": 7, "delta_mm": [100, 0, 0]}
    ]

    stack = assistant._parse_assistant_result(
        json.dumps(
            {
                "message": "Stacked the construction into 20 total layers.",
                "model_intent": {
                    "replace_scene": False,
                    "boxes": [],
                    "linear_arrays": [
                        {
                            "occurrence_ids": list(range(1, 25)),
                            "instances": 20,
                            "step_mm": [0, 0, 280],
                        }
                    ],
                },
            }
        )
    )
    assert stack["model_intent"]["linear_arrays"][0]["instances"] == 20
    assert len(stack["model_intent"]["linear_arrays"][0]["occurrence_ids"]) == 24

    with pytest.raises(assistant.ProtocolError):
        assistant._parse_assistant_result(
            json.dumps(
                {
                    "message": "Too large.",
                    "model_intent": {
                        "replace_scene": False,
                        "boxes": [],
                        "linear_arrays": [
                            {
                                "occurrence_ids": list(range(1, 101)),
                                "instances": 7,
                                "step_mm": [0, 0, 280],
                            }
                        ],
                    },
                }
            )
        )

    for invalid in (
        "not-json",
        json.dumps({"message": "missing intent"}),
        json.dumps(
            {
                "message": "bad",
                "model_intent": {
                    "replace_scene": True,
                    "boxes": [{"name": "Bad", "size_mm": [-1, 1, 1], "origin_mm": [0, 0, 0]}],
                },
            }
        ),
    ):
        with pytest.raises(assistant.ProtocolError):
            assistant._parse_assistant_result(invalid)


def test_public_sidecar_rejects_oauth_secrets_and_arbitrary_capabilities():
    sidecar = assistant.PublicAssistantSidecar(lambda *_: "unused")
    with pytest.raises(assistant.ProtocolError):
        sidecar.handle(hello(distribution="private-oauth"))
    with pytest.raises(assistant.ProtocolError):
        sidecar.handle(hello(api_key="secret"))
    invalid = hello()
    invalid["capabilities"].append("shell")
    with pytest.raises(assistant.ProtocolError):
        sidecar.handle(invalid)
    with pytest.raises(assistant.ProtocolError):
        sidecar.handle(hello(model="../invalid model"))


def test_chat_requires_handshake_and_exact_bounded_fields():
    sidecar = assistant.PublicAssistantSidecar(
        lambda *_: json.dumps({"message": "ok", "model_intent": None})
    )
    chat = {"type": "chat", "request_id": "r1", "message": "hello", "context": {}}
    with pytest.raises(assistant.ProtocolError):
        sidecar.handle(chat)

    sidecar.handle(hello("openai-api"))
    with pytest.raises(assistant.ProtocolError):
        sidecar.handle({**chat, "token": "secret"})
    with pytest.raises(assistant.ProtocolError):
        sidecar.handle({**chat, "message": "x" * (assistant.MAX_MESSAGE_CHARS + 1)})


def test_provider_payloads_never_define_model_tools(monkeypatch):
    requests = []

    def post(url, payload, headers):
        requests.append((url, payload, headers))
        if "anthropic" in url:
            return {"content": [{"type": "text", "text": "Claude answer"}]}
        return {"output": [{"type": "message", "content": [{"type": "output_text", "text": "GPT answer"}]}]}

    monkeypatch.setattr(assistant, "_post_json", post)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-anthropic")
    monkeypatch.setenv("OPENAI_API_KEY", "test-openai")

    assert assistant.send_public_request("anthropic-api", "claude-sonnet-4-6", "hello", ()) == "Claude answer"
    assert assistant.send_public_request("openai-api", "gpt-5.2", "hello", ()) == "GPT answer"
    assert all("tools" not in payload for _, payload, _ in requests)
    assert all("test-" not in json.dumps(payload) for _, payload, _ in requests)
