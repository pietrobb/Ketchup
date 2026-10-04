//! What the AI client sees: server instructions and the tool list.
use serde_json::{Value, json};

pub const INSTRUCTIONS: &str = "\
Kečup is a parametric 3D CAD modeler. These tools drive the Kečup window the user has open; \
every change appears there at once and is one Undo step for the user. Lengths are millimetres.
Tools connect by themselves when exactly one window is open; otherwise call windows and \
connect, or open_window to start one.
Fastest local edit: program action=read mode=source (or selection for the user pick), then \
program action=patch with expected=stamp and edits=[{old,new}]. Reuse loaded context until \
the revision or task changes; read missing context only. apply sends a whole program for a new model. \
Read docs once for needed topics; signatures/rules are default, implementation is opt-in.
Single edits of existing geometry: inspect action=status/summary/query, model \
action=apply_and_verify with typed operations (catalog: inspect action=operations). A typed edit \
of a document that a program owns detaches the program (parameters stop driving the model); \
when program read returns a source, change the model through program apply instead.
Undo and redo: edit action=undo / action=redo, one Undo step per call.
To look at the model use view action=image only when asked or when the result is unclear; \
the validation report of an edit is the check, not a picture.
Every result carries a stamp; pass it back as expected only to guard against concurrent edits \
by the user (required for patch and report). After error connection_lost never repeat a change before reading status.
Preserve the requested join type and simplicity; do not substitute other fasteners without approval. \
Consider transport size, access and assembly order early; offer modular alternatives, do not impose them. \
Geometric contact is not load capacity or FEA verification. Distinguish changed requirements from bugs, \
and unverified/unmeasured from wrong or impossible to measure. A busy response is backend evidence, \
not proof of what the user sees; report its reasons, do not invent window state or cancel human work. \
Report changes and uncertainty briefly; follow report next_offset to retrieve all requested details.";

const EXPECTED: &str = "Optional stamp from an earlier result; the call is rejected with \
stale_document if the document changed since.";

pub fn tools() -> Value {
    let face_targets = json!({"type":"array", "minItems":2, "maxItems":2, "items":{"type":"object", "required":["entity_id","instance_path"], "additionalProperties":false, "properties":{"entity_id":{"type":"integer","minimum":1},"instance_path":{"type":"object","required":["root_occurrence_id","steps"],"properties":{"root_occurrence_id":{"type":"integer","minimum":1},"steps":{"type":"array","items":{"type":"object","required":["kind","owner_definition_id","local_id"],"additionalProperties":false,"properties":{"kind":{"type":"string","enum":["group","occurrence"]},"owner_definition_id":{"type":"integer","minimum":1},"local_id":{"type":"integer","minimum":1}}}}}}}}});
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
    read defaults to source-only; mode=selection returns selected part lines and pick context, mode=full returns the full part map (source null without an owner). \
    apply sends the WHOLE edited program: the window re-evaluates it, rebuilds only the changed parts (unchanged parts keep their IDs) and publishes one Undo step; \
    patch requires expected and unique non-overlapping old/new edits, retains overrides and program ownership, and uses the same atomic apply/validation path. Results give source diff, counts and bounded issue/relation previews; report pages return complete model cut_list, hardware, machining, relations or issues. \
    A rejected program changes nothing and names its line. validate performs a read-only native check of all visible program parts and declared minimum distances, without changing source, selection or Undo history; incomplete checks are not passes. \
    docs returns the library index; with name, concise signatures/rules/example; detail=implementation returns helper code explicitly. Named example files return runnable source. \
    \"This face\"/\"this edge\": inspect action=status -> selected_context.program names the user's pick in program terms.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["read", "apply", "patch", "report", "docs", "validate"]},
                "mode": {"type": "string", "enum": ["source", "selection", "full"], "description": "For read: default source. Selection lines are context, not a dependency closure."},
                "validators": {"type": "array", "items": {"type": "string"}, "description": "For validate: optional existing document validators, e.g. [\"gravity_support\"], with results in validation.document_checks. Program validator \"assembly_path\" checks assembly_step declarations in source order; results in validation.assembly_path. Missing order/paths and unproven contact cannot pass. Program validator \"tool_access\" checks declared auxiliary envelope approaches against visible solids; results in validation.tool_access, no physical tool part is created. Missing envelopes/paths remain incomplete. Does not detach the program."},
                "motion": {"type": "object", "required": ["name", "from", "to"], "additionalProperties": false, "properties": {"name": {"type": "string"}, "from": {"type": "number"}, "to": {"type": "number"}}, "description": "For validate: check a named program motion over the entire interval (mm for slide, degrees for rotation), within its declared limits. Other joints stay at their current poses. Native interval proof; contact, work limits, unsupported bounds or driven parent frames remain incomplete. Results in validation.motion; no document mutation."},
                "edits": {"type": "array", "minItems": 1, "maxItems": 100, "items": {"type": "object", "required": ["old", "new"], "additionalProperties": false, "properties": {"old": {"type": "string", "minLength": 1}, "new": {"type": "string"}}}, "description": "For patch: unique exact text replacements, all matched against the original source. expected is required; stale/ambiguous/overlapping edits change nothing."},
                "section": {"type": "string", "enum": ["cut_list", "hardware", "machining", "relations", "issues"], "description": "For report: required with expected. Cut list has one row per part plus group/group_count; machining one operation per row. Reassemble by group/part. Model-derived, not a prose estimate."},
                "offset": {"type": "integer", "minimum": 0, "description": "For report: default 0; follow next_offset until null at the same expected stamp and section."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "For report: default 50; frame budget may return fewer rows. basis=program_evaluation does not claim a native check."},
                "detail": {"type": "string", "enum": ["concise", "implementation"], "description": "For docs: concise by default; implementation only on explicit request."},
                "source": {"type": "string", "description": "For apply: the complete program text (at most ~32 KB)."},
                "source_path": {"type": "string", "description": "For apply: absolute path of a .star file to send instead of source."},
                "overrides": {"type": "object", "additionalProperties": {"type": "number"}, "description": "For apply: parameter values by name, e.g. {\"width\": 900}."},
                "file_name": {"type": "string", "description": "For apply: program file name for a new program."},
                "replace_document": {"type": "boolean", "description": "For apply: replace a saved document that no program owns."},
                "name": {"type": "string", "description": "For docs: topic id or example file."},
                "expected": {"type": "object", "description": "Required for patch/report: existing stamp from the read/apply result. Optional guard for other actions."},
            }, "allOf": [
                {"if": {"properties": {"action": {"const": "patch"}}}, "then": {"required": ["expected", "edits"]}},
                {"if": {"properties": {"action": {"const": "report"}}}, "then": {"required": ["expected", "section"]}}
            ]},
        },
        {
            "name": "inspect",
            "description": "Read the model. status = document, selection and selected_context; summary = overview; \
    operations = catalog of typed CAD operations for model/edit (operation=<name> for one with all its types); \
    query = one page of occurrences/instances/definitions/features/relations/faces/edges (topology rows carry stable reference IDs and exact geometry); \
    detail = one entity (needs kind and entity_id); measure requires expected plus two current face entity IDs with explicit instance paths; mode=minimum measures trimmed faces, supporting_planes measures signed plane clearance along direction, not finite-face overlap. Read-only, rigid placements only, missing exact evidence is never approximated. workset_create/workset_status = a complete occurrence set for batch.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["status", "summary", "operations", "query", "detail", "measure", "workset_create", "workset_status"]},
                "operation": {"type": "string", "description": "For operations: one operation name, e.g. create_part."}, "faces": face_targets, "mode":{"type":"string","enum":["minimum","supporting_planes"]}, "direction":{"type":"array","minItems":3,"maxItems":3,"items":{"type":"number"}},
                "kind": {"type": "string", "enum": ["occurrences", "instances", "definitions", "features", "relations", "faces", "edges"], "description": "For query, workset_create and detail (required for detail)."},
                "entity_id": {"type": "integer", "minimum": 1, "description": "For detail, with kind."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100},
                "search": {"type": "string", "description": "Name substring, or relation type (uses_definition, member_of_group, assembly_mate)."},
                "definition_id": {"type": "integer", "minimum": 1},
                "tag_id": {"type": "integer", "minimum": 1},
                "classification_dimension_id": {"type": "integer", "minimum": 1},
                "classification_category_id": {"type": "integer", "minimum": 1},
                "world_bounds_mm": {"type": "array", "description": "[[min x,y,z],[max x,y,z]] for instances.", "items": {"type": "array", "items": {"type": "number"}}},
                "cursor": {"type": "string", "description": "Continuation from the previous page."},
                "workset_handle": {"type": "string", "description": "For workset_status."},
                "expected": {"type": "object", "description": "Required for measure: stamp associated with the queried faces. Optional concurrent-edit guard for other actions."},
            }, "allOf": [{"if": {"properties": {"action": {"const": "measure"}}}, "then": {"required": ["expected", "faces", "mode"]}}, {"if": {"properties": {"action": {"const": "measure"}, "mode": {"const": "supporting_planes"}}, "required": ["mode"]}, "then": {"required": ["direction"]}}]},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "model",
            "description": "edit_context returns the editable features, parameters and faces of 1 to 8 parts. \
    apply_and_verify applies one typed CAD program ({\"operations\": [...]}, catalog in inspect action=operations), evaluates exact geometry, runs collision (plus validators) and publishes one Undo step. \
    Issues are reported in validation.issues and the edit stays; fix them with a follow-up edit, or pass strict=true to reject instead. One user request = one apply_and_verify. \
    On a document owned by a program the edit detaches the program (result program_detached=true with a warning; strict=true rejects it instead with program_owned_document); prefer program action=apply there.",
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
            "description": "undo or redo the last change in the window (one Undo step per call; the result says whether a program owns the document again: program_owned), \
    or propose a typed CAD program without applying it and commit that proposal.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["undo", "redo", "propose", "commit"]},
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
            "description": "selection sets the window selection; view sets the camera (iso/top/front are orthographic; iso has Z up; zoom_fit preserves projection; every view frames the whole model); \
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
