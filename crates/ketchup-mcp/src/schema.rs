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
Use list_validators to discover available checks, required inputs, invocation examples and result interpretation. \
The catalog does not run checks or certify the current model.
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
            "name": "list_validators",
            "description": "List all validation capabilities of the connected Kečup window: checks, required inputs/roles, run examples, result paths and limits. Includes document validators, native program checks, motion, assembly paths and tool access. Read-only discovery, not a validation run or a claim that inputs are present. Connects like other window tools; works on empty and non-program documents.",
            "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
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
    patch requires expected and unique non-overlapping old/new edits, retains overrides and program ownership, and uses the same atomic apply/validation path. Results give source diff, counts and bounded issue/relation previews; report pages return complete model cut_list, hardware, machining, relations, issues or material_takeoff (BIM quantities of the visible parts). \
    A rejected program changes nothing and names its line. validate performs a read-only native check of all visible program parts and declared minimum distances, without changing source, selection or Undo history; incomplete checks are not passes. \
    docs returns the library index; with name, concise signatures/rules/example; detail=implementation returns helper code explicitly. Named example files return runnable source. \
    \"This face\"/\"this edge\": inspect action=status -> selected_context.program names the user's pick in program terms.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["read", "apply", "patch", "report", "docs", "validate"]},
                "mode": {"type": "string", "enum": ["source", "selection", "full"], "description": "For read: default source. Selection lines are context, not a dependency closure."},
                "validators": {"type": "array", "items": {"type": "string"}, "description": "For validate: optional existing document validators, e.g. [\"gravity_support\"], with results in validation.document_checks. Program validator \"assembly_path\" checks assembly_step declarations in source order; results in validation.assembly_path. Missing order/paths and unproven contact cannot pass. Program validator \"tool_access\" checks declared auxiliary envelope approaches against visible solids; results in validation.tool_access, no physical tool part is created. Missing envelopes/paths remain incomplete. Does not detach the program."},
                "motion": {"type": "object", "required": ["name", "from", "to"], "additionalProperties": false, "properties": {"name": {"type": "string"}, "from": {"type": "number"}, "to": {"type": "number"}}, "description": "For validate: check a named program motion over the entire interval (mm for slide, degrees for rotation), within its declared limits. Other joints stay at their current poses. Native interval proof; contact, work limits, unsupported bounds or driven parent frames remain incomplete. Results in validation.motion; no document mutation."},
                "edits": {"type": "array", "minItems": 1, "maxItems": 100, "items": {"type": "object", "required": ["old", "new"], "additionalProperties": false, "properties": {"old": {"type": "string", "minLength": 1}, "new": {"type": "string"}}}, "description": "For patch: unique exact text replacements, all matched against the original source. expected is required; stale/ambiguous/overlapping edits change nothing."},
                "section": {"type": "string", "enum": ["cut_list", "hardware", "machining", "relations", "issues", "material_takeoff", "selection_takeoff", "joints", "loads", "members"], "description": "For report: required with expected. Cut list has one row per part plus group/group_count; machining one operation per row. Reassemble by group/part. material_takeoff counts only visible parts (hidden layers excluded): rows by category/material/cross-section with count, length_m, area_m2, volume_m3 and volume_basis (exact solid or blank), then one material_total per material. selection_takeoff is the same for the selected visible parts only: the summary of one wall or floor. joints lists every bearing joint with its published rating (load_n, basis, source), its utilization on that basis (allowable vs characteristic load, characteristic as kmod Rk/1.3, design vs design load) and status pass/fail, or not_verified and what is missing; never a pass without a rating and a load. loads lists every load_path member with characteristic loads by kind (N), patches along it and reactions per support; missing names absent inputs. members lists the EN 1995-1-1 check of every load_path member of a timber_design() material: role, section, checks with utilization, combination and place, the governing utilization and status pass/fail/not_verified (not_verified whenever an input is missing); gravity loads only, wind and horizontal stability are not checked. Model-derived, not a prose estimate."},
                "offset": {"type": "integer", "minimum": 0, "description": "For report: default 0; follow next_offset until null at the same expected stamp and section."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "For report: default 50; frame budget may return fewer rows. basis=program_evaluation does not claim a native check."},
                "detail": {"type": "string", "enum": ["concise", "implementation"], "description": "For docs: concise by default; implementation only on explicit request."},
                "source": {"type": "string", "description": "For apply: the complete program text (at most ~8 MB)."},
                "source_path": {"type": "string", "description": "For apply: absolute path of a .star file to send instead of source."},
                "overrides": {"type": "object", "additionalProperties": {"type": "number"}, "description": "For apply: parameter values by name, e.g. {\"width\": 900}. Omitted keeps the values the window stored for this program (e.g. after Push/Pull); {} clears them."},
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
                "compact": {"type": "boolean", "description": "For occurrence/instance queries: rows carry only id, path, name and visibility (false with hidden_tags when a layer hides the part), so up to 100 fit one page; detail gives the rest."},
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
            "description": "save the document, save_as an absolute path, or open an absolute path in the window (the user confirms in the window when work would be lost). \
    export_drawings writes the project drawings of the visible layers (floor plan cut, longitudinal and cross section, four elevations with overall dimensions) as one vector PDF sheet with a frame and a title block; \
    title_block fields and format given here are kept in the document (an empty string clears a field), scale and format cells are filled in by the sheet. Hidden layers are left out.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["save", "save_as", "open", "export_drawings"]},
                "path": {"type": "string", "description": "For save_as and open; for export_drawings a new local absolute path ending in .pdf (an existing file is never replaced)."},
                "format": {"type": "string", "enum": ["auto", "A3", "A2", "A1", "A0"], "description": "For export_drawings: sheet format; auto picks the smallest sheet that holds the views at 1:50 or finer. Omit to keep the document's format."},
                "title_block": {"type": "object", "description": "For export_drawings: title block values to keep in the document.", "additionalProperties": false, "properties": {
                    "project": {"type": "string"}, "location": {"type": "string"}, "client": {"type": "string"},
                    "drawing": {"type": "string"}, "drawing_number": {"type": "string"}, "stage": {"type": "string"},
                    "date": {"type": "string"}, "job_number": {"type": "string"}, "office": {"type": "string"},
                    "designer": {"type": "string"}, "author": {"type": "string"}, "checked_by": {"type": "string"},
                }},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
        {
            "name": "view",
            "description": "selection sets the window selection; view sets the camera (iso/top/front are orthographic; iso has Z up; zoom_fit preserves projection; every view frames the whole model); \
    image returns a PNG render of the CAD viewport (not a screenshot, not a geometry check). \
    Saved views (stored in the document, never detach a program): saved_views lists them with hidden tag names; \
    save_view stores the current camera, display style and hidden tags under name (same name replaces; one Undo step); \
    show_view name restores one (tag visibility is one Undo step). tag_visibility name visible hides or shows every part of a tag (summary lists tags): \
    one Undo step, no geometry change, no validation, never detaches a program. \
    section cuts the viewport open: everything on the side normal points to is hidden, cut solids show their inside in red; \
    it changes no geometry and no Undo history, and save_view stores it with the view. close_section removes it.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["selection", "view", "image", "saved_views", "save_view", "show_view", "tag_visibility", "section", "close_section"]},
                "name": {"type": "string", "description": "For save_view and show_view: the saved view name; for tag_visibility: the tag name."},
                "visible": {"type": "boolean", "description": "For tag_visibility: true shows the tag, false hides it."},
                "normal": {"type": "array", "items": {"type": "number"}, "minItems": 3, "maxItems": 3, "description": "For section: direction of the hidden side, e.g. [0,0,1] hides everything above."},
                "offset_mm": {"type": "number", "description": "For section: plane distance from the origin along normal, e.g. 1200 cuts at z=1200 for [0,0,1]."},
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
