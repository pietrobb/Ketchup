import hashlib
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


def project_context(document_id=7, exchanges=()):
    entries = []
    for sequence, user, answer in exchanges:
        identity = hashlib.sha256(f"{sequence}\n{user}\n{answer}".encode()).hexdigest()
        entries.append(
            {"sequence": sequence, "user": user, "assistant": answer, "sha256": identity}
        )
    encoded = json.dumps(entries, ensure_ascii=False, separators=(",", ":")).encode()
    return {
        "document_id": document_id,
        "project_memory": {
            "schema": assistant.PROJECT_MEMORY_SCHEMA,
            "document_id": document_id,
            "stored_count": len(entries),
            "retrieved_count": len(entries),
            "complete": True,
            "byte_length": len(encoded),
            "entries": entries,
        },
    }


def tool_context():
    context = project_context()
    context.update(
        {
            "revision": 4,
            "canonical_digest": "a" * 64,
            "selected_occurrence_ids": [7],
            "occurrence_count": 2,
            "occurrences_complete": True,
            "occurrences": [
                {
                    "occurrence_id": 7,
                    "definition_id": 2,
                    "name": "Bottle",
                    "visible": True,
                    "copyable": True,
                    "bounds_mm": {"min": [0, 0, 0], "max": [60, 60, 155]},
                },
                {
                    "occurrence_id": 8,
                    "definition_id": 3,
                    "name": "Cap",
                    "visible": True,
                    "copyable": True,
                    "bounds_mm": {"min": [20, 20, 155], "max": [40, 40, 170]},
                },
            ],
        }
    )
    return context


def test_public_sidecar_exposes_host_collision_report_for_direct_non_mutating_answer():
    context = project_context()
    context["validation"] = {
        "schema": "ketchup.assistant-validation-context.v1",
        "document_id": 7,
        "revision": 4,
        "canonical_digest": "a" * 64,
        "state": "failed",
        "complete": True,
        "visible_occurrence_count": 2,
        "checked_occurrence_count": 2,
        "checked_pair_count": 1,
        "issue_count": 1,
        "issues_complete": True,
        "issues": [
            {
                "code": "collision.detected",
                "severity": "error",
                "evidence_class": "exact",
                "left_occurrence_id": 7,
                "left_name": "Left cabinet",
                "right_occurrence_id": 8,
                "right_name": "Right cabinet",
                "evidence": "left=occurrence:7; right=occurrence:8",
            }
        ],
        "unavailable_occurrences": [],
    }
    calls = []

    def sender(provider, model, message, history):
        calls.append(message)
        assert '"code": "collision.detected"' in message
        assert '"left_occurrence_id": 7' in message
        return json.dumps(
            {
                "message": "Collision: Left cabinet (7) with Right cabinet (8).",
                "model_intent": None,
            }
        )

    sidecar = assistant.PublicAssistantSidecar(sender)
    sidecar.handle(hello())
    result = sidecar.handle(
        {
            "type": "chat",
            "request_id": "validate-1",
            "message": "Check this model for collisions",
            "context": context,
        }
    )
    assert result["message"] == "Collision: Left cabinet (7) with Right cabinet (8)."
    assert result["model_intent"] is None
    assert len(calls) == 1
    assert (
        "validation.collision.state is passed and validation.collision.complete is true"
        in assistant.SYSTEM_PROMPT
    )


def test_public_sidecar_exposes_host_gravity_support_report_for_direct_answer():
    context = project_context()
    context["validation"] = {
        "schema": "ketchup.assistant-validation-context.v1",
        "document_id": 7,
        "revision": 4,
        "canonical_digest": "a" * 64,
        "state": "passed",
        "complete": True,
        "issues": [],
        "unavailable_occurrences": [],
        "gravity_support": {
            "state": "failed",
            "complete": True,
            "gravity_axis": "-Z",
            "floor_z_mm": 0.0,
            "checked_occurrence_count": 2,
            "unsupported_count": 1,
            "issues_complete": True,
            "issues": [
                {
                    "code": "gravity.unsupported",
                    "severity": "warning",
                    "evidence_class": "exact",
                    "occurrence_id": 8,
                    "name": "Floating shelf",
                    "evidence": "body=occurrence:8; gravity_axis=-Z",
                }
            ],
        },
    }
    calls = []

    def sender(provider, model, message, history):
        calls.append(message)
        assert '"code": "gravity.unsupported"' in message
        assert '"name": "Floating shelf"' in message
        return json.dumps(
            {
                "message": "Floating shelf (8) has no support against gravity.",
                "model_intent": None,
            }
        )

    sidecar = assistant.PublicAssistantSidecar(sender)
    sidecar.handle(hello())
    result = sidecar.handle(
        {
            "type": "chat",
            "request_id": "gravity-1",
            "message": "Which parts will fall?",
            "context": context,
        }
    )
    assert result["message"] == "Floating shelf (8) has no support against gravity."
    assert result["model_intent"] is None
    assert len(calls) == 1
    assert "validation.gravity_support.state is passed" in assistant.SYSTEM_PROMPT


@pytest.mark.parametrize("provider", ["anthropic-api", "openai-api"])
def test_public_providers_receive_authoritative_fail_closed_validator_selection(provider):
    context = project_context()
    context["validation"] = {
        "schema": "ketchup.assistant-validation-context.v1",
        "document_id": 7,
        "revision": 4,
        "canonical_digest": "a" * 64,
        "selection_mode": "only",
        "requested": ["collision"],
        "executed": [],
        "skipped": ["collision", "gravity_support"],
        "not_evaluated": [{"validator": "deflection", "reason": "unknown_validator"}],
        "selection_error": "unknown_or_empty_validator_selection",
        "state": "not_evaluated",
        "complete": False,
        "collision": {"state": "skipped", "complete": False, "issues": []},
        "gravity_support": {"state": "skipped", "complete": False, "issues": []},
    }
    calls = []

    def sender(actual_provider, model, message, history):
        calls.append((actual_provider, message))
        assert actual_provider == provider
        assert '"requested": ["collision"]' in message
        assert '"executed": []' in message
        assert '"validator": "deflection"' in message
        return json.dumps(
            {
                "message": "Unknown validator: deflection. No validator was run.",
                "model_intent": None,
            }
        )

    sidecar = assistant.PublicAssistantSidecar(sender)
    sidecar.handle(hello(provider))
    result = sidecar.handle(
        {
            "type": "chat",
            "request_id": "selection-error-1",
            "message": "Only collisions and deflection",
            "context": context,
        }
    )
    assert result["message"] == "Unknown validator: deflection. No validator was run."
    assert result["model_intent"] is None
    assert len(calls) == 1
    assert calls[0][0] == provider
    assert "requested, executed, skipped, and not_evaluated" in assistant.SYSTEM_PROMPT


def test_public_sidecar_conversation_uses_bounded_context_and_no_tools():
    calls = []

    def sender(provider, model, message, history):
        calls.append((provider, model, message, history))
        return json.dumps({"message": "I can prepare a proposal.", "model_intent": None})

    sidecar = assistant.PublicAssistantSidecar(sender)
    ready = sidecar.handle(hello())
    context = project_context()
    result = sidecar.handle(
        {"type": "chat", "request_id": "r1", "message": "Raise it.", "context": context}
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
            f'<document-context>{json.dumps(context, ensure_ascii=False, sort_keys=True)}</document-context>\n\nRaise it.',
            (),
        )
    ]


def test_project_memory_is_bounded_scope_checked_and_tamper_evident():
    context = project_context(7, ((4, "Remember shelf spacing", "The shelf spacing is 320 mm."),))
    calls = []
    sidecar = assistant.PublicAssistantSidecar(
        lambda provider, model, message, history: calls.append(message)
        or json.dumps({"message": "I remember 320 mm.", "model_intent": None})
    )
    sidecar.handle(hello())
    result = sidecar.handle(
        {"type": "chat", "request_id": "memory-1", "message": "What spacing?", "context": context}
    )
    assert result["message"] == "I remember 320 mm."
    assert "Remember shelf spacing" in calls[0]

    invalid_contexts = []
    wrong_scope = project_context(7)
    wrong_scope["project_memory"]["document_id"] = 8
    invalid_contexts.append(wrong_scope)
    wrong_hash = project_context(7, ((1, "Shelf", "320 mm"),))
    wrong_hash["project_memory"]["entries"][0]["sha256"] = "0" * 64
    invalid_contexts.append(wrong_hash)
    oversized = project_context(7, ((1, "x" * 1025, "answer"),))
    invalid_contexts.append(oversized)

    for invalid in invalid_contexts:
        rejected = assistant.PublicAssistantSidecar(
            lambda *_: pytest.fail("invalid memory reached the provider")
        )
        rejected.handle(hello())
        with pytest.raises(assistant.ProtocolError):
            rejected.handle(
                {"type": "chat", "request_id": "bad", "message": "hello", "context": invalid}
            )


def test_public_sidecar_has_no_shell_filesystem_browser_or_agent_authority():
    source = MODULE_PATH.read_text(encoding="utf-8")
    for forbidden in (
        "import subprocess",
        "import pathlib",
        "import shutil",
        "import glob",
        "import socket",
        "playwright",
        "selenium",
        "DocumentStore",
        "claude_engine",
        "peer_memory",
    ):
        assert forbidden not in source


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


def test_public_sidecar_exposes_true_gable_roof_staircase_and_full_slab_opening():
    roof = {
        "name": "True gable roof",
        "length_mm": 5900,
        "span_mm": 4200,
        "rise_mm": 1400,
        "thickness_mm": 180,
        "origin_mm": [-200, -200, 3400],
    }
    stairs = {
        "name": "Attic staircase",
        "run_mm": 3000,
        "width_mm": 800,
        "rise_mm": 3000,
        "step_count": 15,
        "origin_mm": [1800, 2200, 200],
    }
    rafter = {
        "name": "Left rafter",
        "start_mm": [0, -483.05, 3044.07],
        "end_mm": [0, 1900, 4800],
        "up_hint": [0, 0, 1],
        "width_mm": 100,
        "depth_mm": 180,
        "bottom_notches": [{"from_start_mm": 600, "length_mm": 160, "depth_mm": 50}],
    }
    answer = json.dumps(
        {
            "message": "Prepared a true gable roof, attic opening, and staircase.",
            "model_intent": {
                "replace_scene": True,
                "boxes": [
                    {
                        "name": "Attic floor with opening",
                        "size_mm": [5500, 3800, 200],
                        "origin_mm": [0, 0, 3200],
                        "subtract_boxes": [
                            {"size_mm": [900, 1400, 200], "origin_mm": [3900, 1900, 0]}
                        ],
                    }
                ],
                "gable_roofs": [roof],
                "staircases": [stairs],
                "oriented_beams": [rafter],
            },
        }
    )
    result = assistant._parse_assistant_result(answer)["model_intent"]
    assert result["gable_roofs"] == [roof]
    assert result["staircases"] == [stairs]
    assert result["oriented_beams"] == [rafter]
    assert result["boxes"][0]["subtract_boxes"][0]["size_mm"][2] == 200
    assert "never approximate a pitched roof with stepped boxes" in assistant.SYSTEM_PROMPT
    assert "never claim they are unsupported" in assistant.SYSTEM_PROMPT

    for invalid_stairs in (
        {**stairs, "step_count": 1},
        {**stairs, "run_mm": 1000},
        {**stairs, "width_mm": 400},
    ):
        invalid = json.loads(answer)
        invalid["model_intent"]["staircases"] = [invalid_stairs]
        with pytest.raises(assistant.ProtocolError):
            assistant._parse_assistant_result(json.dumps(invalid))

    for invalid_rafter in (
        {**rafter, "end_mm": rafter["start_mm"]},
        {**rafter, "up_hint": [0, 2383.05, 1755.93]},
        {**rafter, "bottom_notches": [{"from_start_mm": 600, "length_mm": 160, "depth_mm": 180}]},
    ):
        invalid = json.loads(answer)
        invalid["model_intent"]["oriented_beams"] = [invalid_rafter]
        with pytest.raises(assistant.ProtocolError):
            assistant._parse_assistant_result(json.dumps(invalid))


def test_public_sidecar_parses_only_bounded_editable_bottle_workflows():
    bottle = {
        "name": "AI ketchup bottle",
        "body_radius_mm": 30,
        "body_height_mm": 110,
        "shoulder_rise_mm": 20,
        "neck_radius_mm": 12,
        "neck_height_mm": 25,
        "wall_thickness_mm": 2,
        "finish_kind": "fillet",
        "finish_amount_mm": 2,
        "origin_mm": [90, 0, 0],
    }
    result = assistant._parse_assistant_result(
        json.dumps(
            {
                "message": "Prepared an editable bottle.",
                "model_intent": {
                    "replace_scene": False,
                    "boxes": [],
                    "bottles": [bottle],
                },
            }
        )
    )
    assert result["model_intent"]["bottles"] == [bottle]
    assert result["model_intent"]["translations"] == []
    assert result["model_intent"]["linear_arrays"] == []

    for change in (
        {"wall_thickness_mm": 6},
        {"finish_amount_mm": 4},
        {"finish_kind": "arbitrary"},
        {"shell_command": "arbitrary"},
    ):
        invalid = {**bottle, **change}
        with pytest.raises(assistant.ProtocolError):
            assistant._parse_assistant_result(
                json.dumps(
                    {
                        "message": "Invalid bottle.",
                        "model_intent": {
                            "replace_scene": False,
                            "boxes": [],
                            "bottles": [invalid],
                        },
                    }
                )
            )


def test_public_sidecar_parses_only_bounded_smooth_teapot_workflows():
    teapot = {
        "name": "Rounded tea pot",
        "body_radius_mm": 70,
        "body_height_mm": 105,
        "shoulder_rise_mm": 22,
        "neck_radius_mm": 42,
        "neck_height_mm": 14,
        "wall_thickness_mm": 3,
        "finish_kind": "fillet",
        "finish_amount_mm": 4,
        "origin_mm": [0, 0, 0],
        "teapot": {
            "handle_clearance_mm": 52,
            "handle_tube_radius_mm": 9,
            "spout_length_mm": 105,
            "spout_radius_mm": 14,
            "lid_height_mm": 18,
            "lid_knob_radius_mm": 10,
        },
    }
    answer = json.dumps(
        {
            "message": "Prepared a smooth hollow tea pot.",
            "model_intent": {
                "replace_scene": False,
                "boxes": [],
                "bottles": [teapot],
            },
        }
    )
    assert assistant._parse_assistant_result(answer)["model_intent"]["bottles"] == [teapot]
    assert "separate removable lid with a locating seat" in assistant.SYSTEM_PROMPT
    assert "never approximate a tea pot or cup with boxes" in assistant.SYSTEM_PROMPT

    invalid = json.loads(answer)
    invalid["model_intent"]["bottles"][0]["teapot"]["spout_radius_mm"] = 2
    with pytest.raises(assistant.ProtocolError):
        assistant._parse_assistant_result(json.dumps(invalid))


def test_public_sidecar_parses_bounded_ketchup_squeeze_bottle():
    bottle = {"name": "Kečup squeeze bottle", "body_radius_mm": 38, "body_height_mm": 145, "shoulder_rise_mm": 28, "neck_radius_mm": 15, "neck_height_mm": 18, "wall_thickness_mm": 2, "finish_kind": "fillet", "finish_amount_mm": 2, "origin_mm": [0, 0, 0], "ketchup_bottle": {"body_depth_ratio": 0.68, "cap_radius_mm": 19.5, "cap_height_mm": 24, "label_width_mm": 58, "label_height_mm": 72, "label_relief_mm": 2.5, "grip_rib_count": 20}}
    answer = json.dumps({"message": "Prepared squeeze bottle.", "model_intent": {"replace_scene": False, "boxes": [], "bottles": [bottle]}})
    assert assistant._parse_assistant_result(answer)["model_intent"]["bottles"] == [bottle]
    invalid = json.loads(answer)
    invalid["model_intent"]["bottles"][0]["ketchup_bottle"]["label_relief_mm"] = 4
    with pytest.raises(assistant.ProtocolError):
        assistant._parse_assistant_result(json.dumps(invalid))


def test_public_sidecar_parses_bounded_balloon_text_with_holes_and_depth():
    text = {
        "name": "Balloon KECUP",
        "text": "KECUP",
        "height_mm": 120,
        "depth_mm": 42,
        "stroke_width_mm": 20,
        "letter_spacing_mm": 12,
        "origin_mm": [25, 10, 5],
    }
    answer = json.dumps({"message": "Prepared balloon letters.", "model_intent": {"replace_scene": False, "boxes": [], "balloon_texts": [text]}})
    assert assistant._parse_assistant_result(answer)["model_intent"]["balloon_texts"] == [text]
    assert "fully rounded in every direction with spherical ends" in assistant.SYSTEM_PROMPT
    assert "never use flat-faced or digital display segments" in assistant.SYSTEM_PROMPT
    invalid = json.loads(answer)
    invalid["model_intent"]["balloon_texts"][0]["stroke_width_mm"] = 40
    with pytest.raises(assistant.ProtocolError):
        assistant._parse_assistant_result(json.dumps(invalid))


def test_public_sidecar_parses_only_one_unmixed_profile_translation():
    translation = {
        "definition_id": 1,
        "body_id": 1,
        "profile_id": 14,
        "delta_mm": [2, 3],
    }
    answer = json.dumps(
        {
            "message": "Moved the selected pocket profile.",
            "model_intent": {
                "replace_scene": False,
                "boxes": [],
                "profile_translations": [translation],
            },
        }
    )
    parsed = assistant._parse_assistant_result(answer)
    assert parsed["model_intent"]["profile_translations"] == [translation]
    assert parsed["model_intent"]["translations"] == []
    context = tool_context()
    context["selected_profile_translation_target"] = {
        "definition_id": 1,
        "body_id": 1,
        "profile_id": 14,
        "name": "Fitting pocket profile",
    }
    message = f'<document-context>{json.dumps(context, sort_keys=True)}</document-context>\n\nMove it.'
    assistant._validate_selected_profile_translation_answer(answer, message)
    mismatched = answer.replace('"profile_id": 14', '"profile_id": 15')
    with pytest.raises(assistant.ProtocolError, match="selected target"):
        assistant._validate_selected_profile_translation_answer(mismatched, message)

    invalid_intents = (
        {"replace_scene": False, "boxes": [], "profile_translations": [{**translation, "delta_mm": [0, 0]}]},
        {"replace_scene": False, "boxes": [], "profile_translations": [{**translation, "profile_id": 0}]},
        {
            "replace_scene": False,
            "boxes": [],
            "translations": [{"occurrence_id": 1, "delta_mm": [1, 0, 0]}],
            "profile_translations": [translation],
        },
    )
    for intent in invalid_intents:
        with pytest.raises(assistant.ProtocolError):
            assistant._parse_assistant_result(
                json.dumps({"message": "Invalid profile move.", "model_intent": intent})
            )


def test_public_sidecar_binds_one_parameter_edit_to_the_selected_exact_target():
    edit = {
        "definition_id": 1,
        "body_id": 1,
        "feature_id": 14,
        "constraint_id": 3,
        "value_mm": 8.5,
    }
    answer = json.dumps(
        {
            "message": "Changed the selected dimension.",
            "model_intent": {
                "replace_scene": False,
                "boxes": [],
                "parameter_edits": [edit],
            },
        }
    )
    parsed = assistant._parse_assistant_result(answer)
    assert parsed["model_intent"]["parameter_edits"] == [edit]
    context = tool_context()
    context["selected_parameter_edit_target"] = {
        "definition_id": 1,
        "body_id": 1,
        "feature_id": 14,
        "constraint_id": 3,
        "name": "Radius 3",
        "current_value_mm": 5.0,
    }
    message = f'<document-context>{json.dumps(context, sort_keys=True)}</document-context>\n\nResize it.'
    assistant._validate_selected_parameter_edit_answer(answer, message)

    mismatched = answer.replace('"constraint_id": 3', '"constraint_id": 4')
    with pytest.raises(assistant.ProtocolError, match="selected target"):
        assistant._validate_selected_parameter_edit_answer(mismatched, message)

    invalid_intents = (
        {"replace_scene": False, "boxes": [], "parameter_edits": [{**edit, "value_mm": 0}]},
        {"replace_scene": False, "boxes": [], "parameter_edits": [{**edit, "constraint_id": 0}]},
        {
            "replace_scene": False,
            "boxes": [],
            "translations": [{"occurrence_id": 1, "delta_mm": [1, 0, 0]}],
            "parameter_edits": [edit],
        },
    )
    for intent in invalid_intents:
        with pytest.raises(assistant.ProtocolError):
            assistant._parse_assistant_result(
                json.dumps({"message": "Invalid parameter edit.", "model_intent": intent})
            )


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


def test_inspect_document_is_revision_bound_bounded_and_fail_closed():
    context = tool_context()
    selected = assistant._inspect_document(
        context, {"scope": "selection", "occurrence_ids": []}
    )
    assert selected == {
        "tool": "inspect_document",
        "document_id": 7,
        "revision": 4,
        "canonical_digest": "a" * 64,
        "scope": "selection",
        "complete": True,
        "occurrence_ids": [7],
        "occurrences": [context["occurrences"][0]],
    }
    explicit = assistant._inspect_document(
        context, {"scope": "occurrences", "occurrence_ids": [8, 7]}
    )
    assert [item["name"] for item in explicit["occurrences"]] == ["Cap", "Bottle"]

    invalid_arguments = (
        {"scope": "selection", "occurrence_ids": [7]},
        {"scope": "occurrences", "occurrence_ids": []},
        {"scope": "occurrences", "occurrence_ids": [7, 7]},
        {"scope": "occurrences", "occurrence_ids": [99]},
        {"scope": "occurrences", "occurrence_ids": [7], "command": "delete"},
    )
    for arguments in invalid_arguments:
        with pytest.raises(assistant.ProtocolError):
            assistant._inspect_document(context, arguments)

    truncated = {**context, "occurrences_complete": False, "occurrences": []}
    with pytest.raises(assistant.ProtocolError, match="absent from the bounded"):
        assistant._inspect_document(
            truncated, {"scope": "occurrences", "occurrence_ids": [7]}
        )


def test_measure_bounds_returns_revision_bound_dimensions_and_clearance_fail_closed():
    context = tool_context()
    measured = assistant._measure_bounds(
        context, {"scope": "occurrences", "occurrence_ids": [7, 8]}
    )
    assert measured["tool"] == "measure_bounds"
    assert measured["document_id"] == 7
    assert measured["revision"] == 4
    assert measured["canonical_digest"] == "a" * 64
    assert measured["measurements"] == [
        {
            "occurrence_id": 7,
            "name": "Bottle",
            "min_mm": [0, 0, 0],
            "max_mm": [60, 60, 155],
            "size_mm": [60, 60, 155],
            "center_mm": [30.0, 30.0, 77.5],
        },
        {
            "occurrence_id": 8,
            "name": "Cap",
            "min_mm": [20, 20, 155],
            "max_mm": [40, 40, 170],
            "size_mm": [20, 20, 15],
            "center_mm": [30.0, 30.0, 162.5],
        },
    ]
    assert measured["pairs"] == [
        {
            "occurrence_ids": [7, 8],
            "axis_gap_mm": [0, 0, 0],
            "clearance_mm": 0.0,
            "touches_or_overlaps": True,
        }
    ]

    separated = tool_context()
    separated["occurrences"][1]["bounds_mm"] = {
        "min": [63, 64, 159],
        "max": [83, 84, 174],
    }
    gap = assistant._measure_bounds(
        separated, {"scope": "occurrences", "occurrence_ids": [7, 8]}
    )["pairs"][0]
    assert gap["axis_gap_mm"] == [3, 4, 4]
    assert gap["clearance_mm"] == pytest.approx(6.403124237)
    assert gap["touches_or_overlaps"] is False

    missing = tool_context()
    missing["occurrences"][0]["bounds_mm"] = None
    with pytest.raises(assistant.ProtocolError, match="no available world bounds"):
        assistant._measure_bounds(missing, {"scope": "selection", "occurrence_ids": []})

    inverted = tool_context()
    inverted["occurrences"][0]["bounds_mm"] = {"min": [1, 0, 0], "max": [0, 1, 1]}
    with pytest.raises(assistant.ProtocolError, match="bounds are invalid"):
        assistant._measure_bounds(inverted, {"scope": "selection", "occurrence_ids": []})

    too_many = tool_context()
    too_many["selected_occurrence_ids"] = list(range(1, 10))
    with pytest.raises(assistant.ProtocolError, match="occurrence limit"):
        assistant._measure_bounds(too_many, {"scope": "selection", "occurrence_ids": []})


def test_plan_placement_returns_revision_bound_translation_and_fails_closed():
    context = tool_context()
    placed = assistant._plan_placement(
        context,
        {
            "moving_occurrence_id": 8,
            "reference_occurrence_id": 7,
            "axis": "z",
            "side": "positive",
            "gap_mm": 2,
            "alignment": "center",
        },
    )
    assert placed == {
        "tool": "plan_placement",
        "document_id": 7,
        "revision": 4,
        "canonical_digest": "a" * 64,
        "complete": True,
        "occurrence_ids": [8, 7],
        "moving_occurrence_id": 8,
        "reference_occurrence_id": 7,
        "axis": "z",
        "side": "positive",
        "gap_mm": 2,
        "alignment": "center",
        "delta_mm": [0.0, 0.0, 2],
        "resulting_bounds_mm": {"min": [20.0, 20.0, 157], "max": [40.0, 40.0, 172]},
    }

    negative = assistant._plan_placement(
        context,
        {
            "moving_occurrence_id": 8,
            "reference_occurrence_id": 7,
            "axis": "x",
            "side": "negative",
            "gap_mm": 5,
            "alignment": "max",
        },
    )
    assert negative["delta_mm"] == [-45, 20, -15]
    assert negative["resulting_bounds_mm"] == {
        "min": [-25, 40, 140],
        "max": [-5, 60, 155],
    }

    valid = {
        "moving_occurrence_id": 8,
        "reference_occurrence_id": 7,
        "axis": "z",
        "side": "positive",
        "gap_mm": 0,
        "alignment": "center",
    }
    invalid_arguments = (
        {**valid, "moving_occurrence_id": 7},
        {**valid, "moving_occurrence_id": 99},
        {**valid, "axis": "arbitrary"},
        {**valid, "axis": []},
        {**valid, "side": "inside"},
        {**valid, "side": []},
        {**valid, "alignment": "arbitrary"},
        {**valid, "alignment": {}},
        {**valid, "gap_mm": -1},
        {**valid, "gap_mm": float("inf")},
        {**valid, "gap_mm": True},
        {**valid, "command": "move"},
    )
    for arguments in invalid_arguments:
        with pytest.raises(assistant.ProtocolError):
            assistant._plan_placement(context, arguments)

    missing_bounds = tool_context()
    missing_bounds["occurrences"][1]["bounds_mm"] = None
    with pytest.raises(assistant.ProtocolError, match="no available world bounds"):
        assistant._plan_placement(missing_bounds, valid)

    out_of_range = tool_context()
    out_of_range["occurrences"][1]["bounds_mm"] = {
        "min": [-1_000_000, 0, 0],
        "max": [-999_999, 1, 1],
    }
    with pytest.raises(assistant.ProtocolError, match="coordinate envelope"):
        assistant._plan_placement(out_of_range, {**valid, "axis": "x"})


def test_plan_linear_array_uses_union_bounds_and_fails_closed():
    context = tool_context()
    planned = assistant._plan_linear_array(
        context,
        {
            "scope": "occurrences",
            "occurrence_ids": [7, 8],
            "axis": "z",
            "direction": "positive",
            "gap_mm": 5,
            "instances": 3,
        },
    )
    assert planned == {
        "tool": "plan_linear_array",
        "document_id": 7,
        "revision": 4,
        "canonical_digest": "a" * 64,
        "complete": True,
        "occurrence_ids": [7, 8],
        "axis": "z",
        "direction": "positive",
        "gap_mm": 5,
        "instances": 3,
        "step_mm": [0.0, 0.0, 175],
        "source_bounds_mm": {"min": [0, 0, 0], "max": [60, 60, 170]},
        "array_bounds_mm": {"min": [0, 0, 0], "max": [60, 60, 520]},
    }

    negative = assistant._plan_linear_array(
        context,
        {
            "scope": "selection",
            "occurrence_ids": [],
            "axis": "x",
            "direction": "negative",
            "gap_mm": 2,
            "instances": 2,
        },
    )
    assert negative["occurrence_ids"] == [7]
    assert negative["step_mm"] == [-62, 0.0, 0.0]
    assert negative["array_bounds_mm"] == {"min": [-62, 0, 0], "max": [60, 60, 155]}

    valid = {
        "scope": "occurrences",
        "occurrence_ids": [7, 8],
        "axis": "z",
        "direction": "positive",
        "gap_mm": 0,
        "instances": 3,
    }
    invalid = (
        {**valid, "occurrence_ids": []},
        {**valid, "axis": "arbitrary"},
        {**valid, "direction": "inside"},
        {**valid, "gap_mm": -1},
        {**valid, "instances": True},
        {**valid, "instances": 1},
        {**valid, "command": "copy"},
    )
    for arguments in invalid:
        with pytest.raises(assistant.ProtocolError):
            assistant._plan_linear_array(context, arguments)

    with pytest.raises(assistant.ProtocolError, match="too many occurrences"):
        assistant._plan_linear_array(context, {**valid, "instances": 258})

    not_copyable = tool_context()
    not_copyable["occurrences"][1]["copyable"] = False
    with pytest.raises(assistant.ProtocolError, match="not copyable"):
        assistant._plan_linear_array(not_copyable, valid)

    missing_bounds = tool_context()
    missing_bounds["occurrences"][1]["bounds_mm"] = None
    with pytest.raises(assistant.ProtocolError, match="no available world bounds"):
        assistant._plan_linear_array(missing_bounds, valid)


def test_provider_payloads_expose_only_the_read_only_inspection_tools(monkeypatch):
    requests = []

    def post(url, payload, headers):
        requests.append((url, payload, headers))
        if "anthropic" in url:
            return {"content": [{"type": "text", "text": "Claude answer"}]}
        return {
            "output": [
                {"type": "message", "content": [{"type": "output_text", "text": "GPT answer"}]}
            ]
        }

    monkeypatch.setattr(assistant, "_post_json", post)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-anthropic")
    monkeypatch.setenv("OPENAI_API_KEY", "test-openai")

    assert (
        assistant.send_public_request("anthropic-api", "claude-sonnet-4-6", "hello", ())
        == "Claude answer"
    )
    assert assistant.send_public_request("openai-api", "gpt-5.2", "hello", ()) == "GPT answer"
    for _, payload, _ in requests:
        assert [tool["name"] for tool in payload["tools"]] == [
            "inspect_document",
            "measure_bounds",
            "plan_placement",
            "plan_linear_array",
        ]
        assert "delete" not in json.dumps(payload["tools"]).lower()
        assert "shell" not in json.dumps(payload["tools"]).lower()
        assert "test-" not in json.dumps(payload)


def test_anthropic_runs_two_distinct_inspection_rounds_then_returns_final_typed_json(monkeypatch):
    requests = []
    final_text = json.dumps(
        {
            "message": "Prepared the exact planned placement.",
            "model_intent": {
                "replace_scene": False,
                "boxes": [],
                "translations": [{"occurrence_id": 8, "delta_mm": [0, 0, 2]}],
            },
        }
    )

    def post(url, payload, headers):
        requests.append(json.loads(json.dumps(payload)))
        if len(requests) <= 2:
            return {
                "content": [
                    {
                        "type": "tool_use",
                        "id": f"tool-{len(requests)}",
                        "name": "inspect_document" if len(requests) == 1 else "plan_placement",
                        "input": (
                            {"scope": "selection", "occurrence_ids": []}
                            if len(requests) == 1
                            else {
                                "moving_occurrence_id": 8,
                                "reference_occurrence_id": 7,
                                "axis": "z",
                                "side": "positive",
                                "gap_mm": 2,
                                "alignment": "center",
                            }
                        ),
                    }
                ]
            }
        return {"content": [{"type": "text", "text": final_text}]}

    monkeypatch.setattr(assistant, "_post_json", post)
    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-anthropic")
    context = tool_context()
    message = (
        f'<document-context>{json.dumps(context, ensure_ascii=False, sort_keys=True)}</document-context>\n\n'
        "How tall is the selection?"
    )

    assert (
        assistant.send_public_request("anthropic-api", "claude-sonnet-4-6", message, ())
        == final_text
    )
    assert len(requests) == 3
    first_receipt = requests[1]["messages"][-1]["content"][0]
    assert first_receipt["type"] == "tool_result"
    assert first_receipt["tool_use_id"] == "tool-1"
    assert json.loads(first_receipt["content"])["occurrences"][0]["name"] == "Bottle"
    assert requests[2]["messages"][:-2] == requests[1]["messages"]
    assert requests[2]["messages"][-2]["content"][0]["id"] == "tool-2"
    second_receipt = requests[2]["messages"][-1]["content"][0]
    assert second_receipt["tool_use_id"] == "tool-2"
    assert json.loads(second_receipt["content"])["delta_mm"] == [0.0, 0.0, 2]
    assert assistant._parse_assistant_result(final_text)["model_intent"]["translations"] == [
        {"occurrence_id": 8, "delta_mm": [0, 0, 2]}
    ]


def test_anthropic_inspection_rounds_enforce_fail_closed_limits(monkeypatch):
    context = tool_context()
    message = (
        f'<document-context>{json.dumps(context, ensure_ascii=False, sort_keys=True)}</document-context>\n\n'
        "Inspect the selection, then occurrence 8."
    )
    queries = (
        {"scope": "selection", "occurrence_ids": []},
        {"scope": "occurrences", "occurrence_ids": [8]},
    )

    def scripted_calls(arguments, call_ids):
        attempt = 0

        def scripted_post(url, payload, headers):
            nonlocal attempt
            index = attempt
            attempt += 1
            return {
                "content": [
                    {
                        "type": "tool_use",
                        "id": call_ids[index],
                        "name": "inspect_document",
                        "input": arguments[index],
                    }
                ]
            }

        return scripted_post

    monkeypatch.setenv("ANTHROPIC_API_KEY", "test-anthropic")
    failure_cases = (
        (
            (queries[0], {"scope": "occurrences", "occurrence_ids": [7]}),
            ("tool-1", "tool-2"),
            "repeated.*query",
        ),
        ((queries[0], queries[1]), ("tool-1", "tool-1"), "repeated.*call ID"),
        (
            (queries[0], queries[1], {"scope": "occurrences", "occurrence_ids": [7]}),
            ("tool-1", "tool-2", "tool-3"),
            "inspection limit",
        ),
    )
    for arguments, call_ids, error in failure_cases:
        monkeypatch.setattr(assistant, "_post_json", scripted_calls(arguments, call_ids))
        with pytest.raises(assistant.ProtocolError, match=error):
            assistant.send_public_request("anthropic-api", "claude-sonnet-4-6", message, ())

    def parallel_calls(url, payload, headers):
        return {
            "content": [
                {
                    "type": "tool_use",
                    "id": call_id,
                    "name": "inspect_document",
                    "input": query,
                }
                for call_id, query in zip(("tool-1", "tool-2"), queries)
            ]
        }

    monkeypatch.setattr(assistant, "_post_json", parallel_calls)
    with pytest.raises(assistant.ProtocolError, match="more than one"):
        assistant.send_public_request("anthropic-api", "claude-sonnet-4-6", message, ())


def test_openai_runs_two_distinct_inspection_rounds_and_enforces_limits(monkeypatch):
    requests = []
    final_text = json.dumps(
        {
            "message": "Prepared the exact planned placement.",
            "model_intent": {
                "replace_scene": False,
                "boxes": [],
                "translations": [{"occurrence_id": 8, "delta_mm": [0, 0, 2]}],
            },
        }
    )
    queries = (
        {"scope": "selection", "occurrence_ids": []},
        {"scope": "occurrences", "occurrence_ids": [8]},
    )

    def post(url, payload, headers):
        requests.append(json.loads(json.dumps(payload)))
        if len(requests) <= 2:
            return {
                "output": [
                    {
                        "type": "function_call",
                        "call_id": f"call-{len(requests)}",
                        "name": "inspect_document" if len(requests) == 1 else "plan_placement",
                        "arguments": json.dumps(
                            queries[0]
                            if len(requests) == 1
                            else {
                                "moving_occurrence_id": 8,
                                "reference_occurrence_id": 7,
                                "axis": "z",
                                "side": "positive",
                                "gap_mm": 2,
                                "alignment": "center",
                            }
                        ),
                    }
                ]
            }
        return {
            "output": [
                {"type": "message", "content": [{"type": "output_text", "text": final_text}]}
            ]
        }

    monkeypatch.setattr(assistant, "_post_json", post)
    monkeypatch.setenv("OPENAI_API_KEY", "test-openai")
    context = tool_context()
    message = (
        f'<document-context>{json.dumps(context, ensure_ascii=False, sort_keys=True)}</document-context>\n\n'
        "Inspect the selection, then occurrence 8."
    )

    assert assistant.send_public_request("openai-api", "gpt-5.2", message, ()) == final_text
    assert len(requests) == 3
    first_receipt = requests[1]["input"][-1]
    assert first_receipt["type"] == "function_call_output"
    assert json.loads(first_receipt["output"])["occurrences"][0]["name"] == "Bottle"
    assert requests[2]["input"][:-2] == requests[1]["input"]
    second_receipt = requests[2]["input"][-1]
    assert second_receipt["call_id"] == "call-2"
    assert json.loads(second_receipt["output"])["delta_mm"] == [0.0, 0.0, 2]

    def scripted_calls(arguments, call_ids):
        attempt = 0

        def scripted_post(url, payload, headers):
            nonlocal attempt
            index = attempt
            attempt += 1
            return {
                "output": [
                    {
                        "type": "function_call",
                        "call_id": call_ids[index],
                        "name": "inspect_document",
                        "arguments": json.dumps(arguments[index]),
                    }
                ]
            }

        return scripted_post

    failure_cases = (
        (
            (queries[0], {"scope": "occurrences", "occurrence_ids": [7]}),
            ("call-1", "call-2"),
            "repeated.*query",
        ),
        ((queries[0], queries[1]), ("call-1", "call-1"), "repeated.*call ID"),
        (
            (queries[0], queries[1], {"scope": "occurrences", "occurrence_ids": [7]}),
            ("call-1", "call-2", "call-3"),
            "inspection limit",
        ),
    )
    for arguments, call_ids, error in failure_cases:
        monkeypatch.setattr(assistant, "_post_json", scripted_calls(arguments, call_ids))
        with pytest.raises(assistant.ProtocolError, match=error):
            assistant.send_public_request("openai-api", "gpt-5.2", message, ())

    def parallel_calls(url, payload, headers):
        return {
            "output": [
                {
                    "type": "function_call",
                    "call_id": call_id,
                    "name": "inspect_document",
                    "arguments": json.dumps(query),
                }
                for call_id, query in zip(("call-1", "call-2"), queries)
            ]
        }

    monkeypatch.setattr(assistant, "_post_json", parallel_calls)
    with pytest.raises(assistant.ProtocolError, match="more than one"):
        assistant.send_public_request("openai-api", "gpt-5.2", message, ())


@pytest.mark.parametrize(
    ("provider", "model", "key_name"),
    (
        ("anthropic-api", "claude-sonnet-4-6", "ANTHROPIC_API_KEY"),
        ("openai-api", "gpt-5.2", "OPENAI_API_KEY"),
    ),
)
@pytest.mark.parametrize(
    "model_intent",
    (
        {
            "replace_scene": False,
            "boxes": [],
            "translations": [{"occurrence_id": 8, "delta_mm": [0, 0, 3]}],
        },
        {
            "replace_scene": False,
            "boxes": [],
            "translations": [{"occurrence_id": 7, "delta_mm": [0, 0, 2]}],
        },
        {
            "replace_scene": False,
            "boxes": [{"name": "Extra", "size_mm": [1, 1, 1], "origin_mm": [0, 0, 0]}],
            "translations": [{"occurrence_id": 8, "delta_mm": [0, 0, 2]}],
        },
        {
            "replace_scene": False,
            "boxes": [],
            "translations": [
                {"occurrence_id": 8, "delta_mm": [0, 0, 2]},
                {"occurrence_id": 9, "delta_mm": [1, 0, 0]},
            ],
        },
    ),
)
def test_provider_rejects_mutations_that_do_not_exactly_match_plan_placement(
    monkeypatch, provider, model, key_name, model_intent
):
    calls = 0
    final_text = json.dumps({"message": "Prepared placement.", "model_intent": model_intent})
    plan_arguments = {
        "moving_occurrence_id": 8,
        "reference_occurrence_id": 7,
        "axis": "z",
        "side": "positive",
        "gap_mm": 2,
        "alignment": "center",
    }

    def post(url, payload, headers):
        nonlocal calls
        calls += 1
        if calls == 1 and provider == "anthropic-api":
            return {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "placement-1",
                        "name": "plan_placement",
                        "input": plan_arguments,
                    }
                ]
            }
        if calls == 1:
            return {
                "output": [
                    {
                        "type": "function_call",
                        "call_id": "placement-1",
                        "name": "plan_placement",
                        "arguments": json.dumps(plan_arguments),
                    }
                ]
            }
        if provider == "anthropic-api":
            return {"content": [{"type": "text", "text": final_text}]}
        return {
            "output": [
                {"type": "message", "content": [{"type": "output_text", "text": final_text}]}
            ]
        }

    monkeypatch.setattr(assistant, "_post_json", post)
    monkeypatch.setenv(key_name, "test-key")
    context = tool_context()
    message = (
        f'<document-context>{json.dumps(context, ensure_ascii=False, sort_keys=True)}</document-context>\n\n'
        "Place the cap 2 mm above the bottle."
    )
    with pytest.raises(assistant.ProtocolError, match="planned placement"):
        assistant.send_public_request(provider, model, message, ())


@pytest.mark.parametrize(
    ("provider", "model", "key_name"),
    (
        ("anthropic-api", "claude-sonnet-4-6", "ANTHROPIC_API_KEY"),
        ("openai-api", "gpt-5.2", "OPENAI_API_KEY"),
    ),
)
def test_provider_accepts_exact_host_planned_linear_array(
    monkeypatch, provider, model, key_name
):
    requests = []
    arguments = {
        "scope": "occurrences",
        "occurrence_ids": [7, 8],
        "axis": "z",
        "direction": "positive",
        "gap_mm": 5,
        "instances": 3,
    }
    final_text = json.dumps(
        {
            "message": "Prepared three exact assembly layers.",
            "model_intent": {
                "replace_scene": False,
                "boxes": [],
                "linear_arrays": [
                    {"occurrence_ids": [7, 8], "instances": 3, "step_mm": [0, 0, 175]}
                ],
            },
        }
    )

    def post(url, payload, headers):
        requests.append(json.loads(json.dumps(payload)))
        if len(requests) == 1 and provider == "anthropic-api":
            return {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "array-1",
                        "name": "plan_linear_array",
                        "input": arguments,
                    }
                ]
            }
        if len(requests) == 1:
            return {
                "output": [
                    {
                        "type": "function_call",
                        "call_id": "array-1",
                        "name": "plan_linear_array",
                        "arguments": json.dumps(arguments),
                    }
                ]
            }
        if provider == "anthropic-api":
            return {"content": [{"type": "text", "text": final_text}]}
        return {
            "output": [
                {"type": "message", "content": [{"type": "output_text", "text": final_text}]}
            ]
        }

    monkeypatch.setattr(assistant, "_post_json", post)
    monkeypatch.setenv(key_name, "test-key")
    context = tool_context()
    message = (
        f'<document-context>{json.dumps(context, ensure_ascii=False, sort_keys=True)}</document-context>\n\n'
        "Stack the bottle and cap into three total layers with 5 mm gaps."
    )
    assert assistant.send_public_request(provider, model, message, ()) == final_text
    receipt = (
        requests[1]["messages"][-1]["content"][0]["content"]
        if provider == "anthropic-api"
        else requests[1]["input"][-1]["output"]
    )
    assert json.loads(receipt)["step_mm"] == [0.0, 0.0, 175]


@pytest.mark.parametrize(
    ("provider", "model", "key_name"),
    (
        ("anthropic-api", "claude-sonnet-4-6", "ANTHROPIC_API_KEY"),
        ("openai-api", "gpt-5.2", "OPENAI_API_KEY"),
    ),
)
@pytest.mark.parametrize(
    "model_intent",
    (
        {
            "replace_scene": False,
            "boxes": [],
            "linear_arrays": [
                {"occurrence_ids": [8, 7], "instances": 3, "step_mm": [0, 0, 175]}
            ],
        },
        {
            "replace_scene": False,
            "boxes": [],
            "linear_arrays": [
                {"occurrence_ids": [7, 8], "instances": 4, "step_mm": [0, 0, 175]}
            ],
        },
        {
            "replace_scene": False,
            "boxes": [],
            "linear_arrays": [
                {"occurrence_ids": [7, 8], "instances": 3, "step_mm": [0, 0, 170]}
            ],
        },
        {
            "replace_scene": False,
            "boxes": [],
            "translations": [{"occurrence_id": 7, "delta_mm": [1, 0, 0]}],
            "linear_arrays": [
                {"occurrence_ids": [7, 8], "instances": 3, "step_mm": [0, 0, 175]}
            ],
        },
    ),
)
def test_provider_rejects_mutations_that_do_not_exactly_match_planned_linear_array(
    monkeypatch, provider, model, key_name, model_intent
):
    calls = 0
    arguments = {
        "scope": "occurrences",
        "occurrence_ids": [7, 8],
        "axis": "z",
        "direction": "positive",
        "gap_mm": 5,
        "instances": 3,
    }
    final_text = json.dumps({"message": "Prepared array.", "model_intent": model_intent})

    def post(url, payload, headers):
        nonlocal calls
        calls += 1
        if calls == 1 and provider == "anthropic-api":
            return {
                "content": [
                    {
                        "type": "tool_use",
                        "id": "array-1",
                        "name": "plan_linear_array",
                        "input": arguments,
                    }
                ]
            }
        if calls == 1:
            return {
                "output": [
                    {
                        "type": "function_call",
                        "call_id": "array-1",
                        "name": "plan_linear_array",
                        "arguments": json.dumps(arguments),
                    }
                ]
            }
        if provider == "anthropic-api":
            return {"content": [{"type": "text", "text": final_text}]}
        return {
            "output": [
                {"type": "message", "content": [{"type": "output_text", "text": final_text}]}
            ]
        }

    monkeypatch.setattr(assistant, "_post_json", post)
    monkeypatch.setenv(key_name, "test-key")
    context = tool_context()
    message = (
        f'<document-context>{json.dumps(context, ensure_ascii=False, sort_keys=True)}</document-context>\n\n'
        "Stack the bottle and cap into three total layers with 5 mm gaps."
    )
    with pytest.raises(assistant.ProtocolError, match="planned (linear )?array"):
        assistant.send_public_request(provider, model, message, ())
