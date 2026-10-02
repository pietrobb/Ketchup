//! What the AI client sees: server instructions and the tool list.
use serde_json::{Value, json};

pub const INSTRUCTIONS: &str = "\
Kečup is a parametric 3D CAD modeler. These tools drive the Kečup window the user has open; \
every change appears there at once and is one Undo step for the user. Lengths are millimetres.
Tools connect by themselves when exactly one window is open; otherwise call windows and \
connect, or open_window to start one.
Fastest way to model: program action=read, then program action=apply with the WHOLE edited \
Starlark program. For a new model first read program action=docs (topics basics and \
placement, then one of the listed examples).
Single edits of existing geometry: inspect action=status/summary/query, model \
action=apply_and_verify with typed operations (catalog: inspect action=operations).
To look at the model use view action=image only when asked or when the result is unclear; \
the validation report of an edit is the check, not a picture.
Every result carries a stamp; pass it back as expected only to guard against concurrent edits \
by the user. After error connection_lost never repeat a change before reading status.";

const EXPECTED: &str = "Optional stamp from an earlier result; the call is rejected with \
stale_document if the document changed since.";

pub fn tools() -> Value {
    json!([
        {
            "name": "windows",
            "description": "List the open Kečup windows (instance_id, document) and which one this server is connected to.",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "connect",
            "description": "Connect to an open Kečup window. Without instance_id connects to the only open window. Replaces any other client connected to that window.",
            "inputSchema": {"type": "object", "properties": {
                "instance_id": {"type": "string", "description": "32-hex window ID from windows."},
            }},
        },
        {
            "name": "open_window",
            "description": "Start a new Kečup window (optionally opening a document) and connect to it. Use only when no window is open or the user asks for a new one.",
            "inputSchema": {"type": "object", "properties": {
                "document_path": {"type": "string", "description": "Optional absolute path of an existing .ketchup document."},
            }},
        },
        {
            "name": "program",
            "description": "Model by editing the window's Starlark program: the fastest path. \
    read returns {source, overrides, parts:[{name, occurrence_id, lines}]} (source null when no program owns the document). \
    apply sends the WHOLE edited program: the window re-evaluates it, rebuilds only the changed parts (unchanged parts keep their IDs) and publishes one Undo step; \
    the result lists added/removed parts and the report (issues = collisions, missing contacts, ...; relations = touching faces, overlaps, gaps within 20 mm). \
    A rejected program changes nothing and names its line. \
    docs returns the program library index (topics with their helpers for parts, placement, profiles, machining and joints; example programs); with name, one topic or example in full. \
    \"This face\"/\"this edge\": inspect action=status -> selected_context.program names the user's pick in program terms.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["read", "apply", "docs"]},
                "source": {"type": "string", "description": "For apply: the complete program text (at most ~32 KB)."},
                "source_path": {"type": "string", "description": "For apply: absolute path of a .star file to send instead of source."},
                "overrides": {"type": "object", "additionalProperties": {"type": "number"}, "description": "For apply: parameter values by name, e.g. {\"width\": 900}."},
                "file_name": {"type": "string", "description": "For apply: program file name for a new program."},
                "replace_document": {"type": "boolean", "description": "For apply: replace a saved document that no program owns."},
                "name": {"type": "string", "description": "For docs: topic id or example file."},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
        {
            "name": "inspect",
            "description": "Read the model. status = document, selection and selected_context; summary = overview; \
    operations = catalog of typed CAD operations for model/edit (operation=<name> for one with all its types); \
    query = one page of occurrences/instances/definitions/features/relations/faces/edges (topology rows carry stable reference IDs and exact geometry); \
    detail = one entity; workset_create/workset_status = a complete occurrence set for batch.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["status", "summary", "operations", "query", "detail", "workset_create", "workset_status"]},
                "operation": {"type": "string", "description": "For operations: one operation name, e.g. create_part."},
                "kind": {"type": "string", "enum": ["occurrences", "instances", "definitions", "features", "relations", "faces", "edges"]},
                "entity_id": {"type": "integer", "minimum": 1, "description": "For detail."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                "search": {"type": "string", "description": "Name substring, or relation type (uses_definition, member_of_group, assembly_mate)."},
                "definition_id": {"type": "integer", "minimum": 1},
                "tag_id": {"type": "integer", "minimum": 1},
                "classification_dimension_id": {"type": "integer", "minimum": 1},
                "classification_category_id": {"type": "integer", "minimum": 1},
                "world_bounds_mm": {"type": "array", "description": "[[min x,y,z],[max x,y,z]] for instances.", "items": {"type": "array", "items": {"type": "number"}}},
                "cursor": {"type": "string", "description": "Continuation from the previous page."},
                "workset_handle": {"type": "string", "description": "For workset_status."},
                "expected": {"type": "object", "description": EXPECTED},
            }},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "model",
            "description": "edit_context returns the editable features, parameters and faces of 1 to 8 parts. \
    apply_and_verify applies one typed CAD program ({\"operations\": [...]}, catalog in inspect action=operations), evaluates exact geometry, runs collision (plus validators) and publishes one Undo step. \
    Issues are reported in validation.issues and the edit stays; fix them with a follow-up edit, or pass strict=true to reject instead. One user request = one apply_and_verify.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["edit_context", "apply_and_verify"]},
                "targets": {"type": "array", "items": {"type": "object"}, "description": "For edit_context: instance paths {root_occurrence_id, steps: []}."},
                "program": {"type": "object", "description": "For apply_and_verify: {\"operations\": [...]}."},
                "selection": {"type": "array", "items": {"type": "integer"}, "description": "Optional root occurrence IDs the window selection must equal."},
                "validators": {"type": "array", "items": {"type": "string"}, "description": "Extra validators, e.g. [\"gravity_support\"]."},
                "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 120000},
                "save": {"type": "object", "description": "Optional {\"mode\":\"current\"} or {\"mode\":\"path\",\"path\":\"...\"}."},
                "strict": {"type": "boolean"},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
        {
            "name": "edit",
            "description": "propose a typed CAD program without applying it, commit a proposal, undo or redo.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["propose", "commit", "undo", "redo"]},
                "program": {"type": "object", "description": "For propose: {\"operations\": [...]}."},
                "selection": {"type": "array", "items": {"type": "integer"}},
                "proposal_id": {"type": "integer", "minimum": 1, "description": "For commit."},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
        {
            "name": "file",
            "description": "save the document, save_as an absolute path, or open an absolute path in the window (the user confirms in the window when work would be lost).",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["save", "save_as", "open"]},
                "path": {"type": "string", "description": "For save_as and open."},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
        {
            "name": "view",
            "description": "selection sets the window selection; view sets the camera (iso, top, front, zoom_fit); \
    image returns a PNG render of the CAD viewport (not a screenshot, not a geometry check).",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["selection", "view", "image"]},
                "occurrence_ids": {"type": "array", "items": {"type": "integer"}, "description": "For selection; [] clears it."},
                "view": {"type": "string", "enum": ["iso", "top", "front", "zoom_fit"]},
                "max_side_px": {"type": "integer", "minimum": 512, "maximum": 1600},
                "framing": {"type": "string", "enum": ["viewport", "selection", "detail_selection"]},
                "capture_mode": {"type": "string", "enum": ["offscreen", "visible_viewport"]},
                "detail_occurrence_id": {"type": "integer", "description": "For detail_selection."},
                "detail_kind": {"type": "string", "enum": ["edges", "faces"]},
                "detail_entity_id": {"type": "integer", "description": "For detail_selection: ID from a faces/edges query."},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
        {
            "name": "batch",
            "description": "Run a bounded job over a workset (inspect action=workset_create), one step per call: start, status, step, cancel.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["start", "status", "step", "cancel"]},
                "workset_handle": {"type": "string", "description": "For start."},
                "operation": {"type": "object", "description": "For start, e.g. {\"type\":\"set_color\",\"color\":[r,g,b]}."},
                "job_handle": {"type": "string", "description": "For status, step and cancel."},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
    ])
}
