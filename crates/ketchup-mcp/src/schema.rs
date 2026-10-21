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
of a document that a program owns detaches the program (parameters stop driving the model), \
except a transform, delete, set_color, mirror or set_feature_parameter of parts the program makes, which is written into the program; \
when program read returns a source, change the model through program apply instead.
Undo and redo: edit action=undo / action=redo, one Undo step per call.
Files: file action=import brings STEP/STP, IGES, STL, DXF, DWG, GLB and SketchUp scene files into the document; \
file action=export writes STEP, IGES, STL, 3MF, GLB, DXF, Viewer, cut-list CSV, HOMAG MPR or BTLx; \
export_drawings writes the project drawings as PDF, or a drawing sheet of the document (sheet_id) as DXF or DWG; save/save_as/open handle .ketchup documents.
To look at the model use view action=image only when asked or when the result is unclear; \
the validation report of an edit is the check, not a picture.
Every result carries a stamp; pass it back as expected only to guard against concurrent edits \
by the user (required for patch and report). After error connection_lost never repeat a change before reading status. \
After connection_taken_over another AI client holds the window: connect again only if the user wants it back.
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
    In a large program read pieces instead: mode=outline lists top-level defs, params and comment sections with their lines; lines=[first,last], search=<text> (case-insensitive) or part=<name> return just those numbered lines. \
    set_params with expected and params={name: value} re-applies the current source with those parameter values (others keep their stored values), without sending the source. \
    save_variant with expected and name keeps every current parameter value as a named variant of the program; use_variant re-applies one like set_params (one Undo step; refused, naming them, when the program no longer declares some of its parameters); delete_variant removes one; read lists them under variants with the active one. \
    export_variants with expected and path (a new local .csv or .xlsx) evaluates every variant apart from the window and writes their cut lists as one table with the variant first, and their material takeoffs beside it as <name>.takeoff.<ext>; existing files are never replaced, the document and its Undo history stay unchanged, and the answer lists each variant's values and material totals. \
    apply sends the WHOLE edited program: the window re-evaluates it, rebuilds only the changed parts (unchanged parts keep their IDs) and publishes one Undo step; \
    patch requires expected and unique non-overlapping old/new edits, retains overrides and program ownership, and uses the same atomic apply/validation path. Results give source diff, counts and bounded issue/relation previews; report pages return complete model cut_list, hardware, bar_schedule, machining, relations, issues or material_takeoff (BIM quantities of the visible parts). \
    check takes the same source/source_path or expected+edits as apply/patch and answers the same way (change, added/removed, report, exact collisions) without publishing anything: check_only=true, canonical_mutation=false. \
    A rejected program changes nothing and names its line. validate performs a read-only native check of all visible program parts and declared minimum distances, without changing source, selection or Undo history; incomplete checks are not passes. \
    docs returns the library index; with name, concise signatures/rules/example; detail=implementation returns helper code explicitly. Named example files return runnable source. \
    \"This face\"/\"this edge\": inspect action=status -> selected_context.program names the user's pick in program terms.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["read", "apply", "patch", "check", "set_params", "save_variant", "use_variant", "delete_variant", "export_variants", "report", "docs", "validate"]},
                "mode": {"type": "string", "enum": ["source", "selection", "outline", "full"], "description": "For read: default source. Selection lines are context, not a dependency closure. outline lists defs, params and comment sections with line numbers."},
                "lines": {"type": "array", "items": {"type": "integer", "minimum": 1}, "minItems": 2, "maxItems": 2, "description": "For read: inclusive 1-based line range [first, last] of the source (at most 400 lines per answer)."},
                "search": {"type": "string", "minLength": 1, "description": "For read: the source lines containing this text, ignoring case."},
                "part": {"type": "string", "description": "For read: the source lines that define this program part, with two neighbours."},
                "params": {"type": "object", "additionalProperties": {"type": "number"}, "description": "For set_params: new values of declared parameters by name, e.g. {\"width\": 900}."},
                "validators": {"type": "array", "items": {"type": "string"}, "description": "For validate: optional existing document validators, e.g. [\"gravity_support\"], with results in validation.document_checks. Program validator \"assembly_path\" checks assembly_step declarations in source order; results in validation.assembly_path. Missing order/paths and unproven contact cannot pass. Program validator \"tool_access\" checks declared auxiliary envelope approaches against visible solids; results in validation.tool_access, no physical tool part is created. Missing envelopes/paths remain incomplete. Does not detach the program."},
                "motion": {"type": "object", "required": ["name", "from", "to"], "additionalProperties": false, "properties": {"name": {"type": "string"}, "from": {"type": "number"}, "to": {"type": "number"}}, "description": "For validate: check a named program motion over the entire interval (mm for slide, degrees for rotation), within its declared limits. Other joints stay at their current poses. Native interval proof; contact, work limits, unsupported bounds or driven parent frames remain incomplete. Results in validation.motion; no document mutation."},
                "edits": {"type": "array", "minItems": 1, "maxItems": 100, "items": {"type": "object", "required": ["old", "new"], "additionalProperties": false, "properties": {"old": {"type": "string", "minLength": 1}, "new": {"type": "string"}}}, "description": "For patch and check: unique exact text replacements, all matched against the original source. expected is required; stale/ambiguous/overlapping edits change nothing."},
                "section": {"type": "string", "enum": ["cut_list", "hardware", "bar_schedule", "machining", "relations", "issues", "material_takeoff", "selection_takeoff", "joints", "loads", "members"], "description": "For report: required with expected. Cut list has one row per made part plus group/group_count; hardware one row per bought item (a joint's fastener, or the parts with a designation attribute: item = designation, count, parts, attributes); bar_schedule one row per position of bars cut (and bent) to length, e.g. rebar (the parts with a cut_length_mm attribute: position, diameter_mm, grade, shape, cut_length_mm, count, total_length_m, mass_kg derived, parts); neither is in the cut list; machining one operation per row. Reassemble by group/part. Cut list, hardware, bar schedule, machining and takeoffs count only the parts of production(tags=…) when the program declares it, shown or hidden. Without it material_takeoff counts only visible parts (hidden layers excluded): rows by category/material/cross-section with count, length_m, area_m2, volume_m3 and volume_basis (exact solid or blank), then one material_total per material. selection_takeoff is the same for the selected visible parts only: the summary of one wall or floor. joints lists every bearing joint, and every joint holding a member down, with its published rating (load_n, tension_n, basis, source), its pull (tension, checked against tension_n), its utilization on that basis (allowable vs characteristic load, characteristic as kmod Rk/1.3, design vs design load) and status pass/fail, or not_verified and what is missing; never a pass without a rating and a load. loads lists every load_path member with characteristic loads by kind (N), patches along it and reactions per support; missing names absent inputs. members lists the EN 1995-1-1 check of every load_path member of a timber_design() material: role, section, checks with utilization, combination and place, the governing utilization and status pass/fail/not_verified (not_verified whenever an input is missing); gravity loads only, wind and horizontal stability are not checked. Model-derived, not a prose estimate."},
                "offset": {"type": "integer", "minimum": 0, "description": "For report: default 0; follow next_offset until null at the same expected stamp and section."},
                "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "For report: default 50; frame budget may return fewer rows. basis=program_evaluation does not claim a native check."},
                "detail": {"type": "string", "enum": ["concise", "implementation"], "description": "For docs: concise by default; implementation only on explicit request."},
                "source": {"type": "string", "description": "For apply and check: the complete program text (at most ~8 MB)."},
                "source_path": {"type": "string", "description": "For apply and check: absolute path of a .star file to send instead of source."},
                "overrides": {"type": "object", "additionalProperties": {"type": "number"}, "description": "For apply and check: parameter values by name, e.g. {\"width\": 900}. Omitted keeps the values the window stored for this program (e.g. after Push/Pull); {} clears them."},
                "file_name": {"type": "string", "description": "For apply: program file name for a new program."},
                "path": {"type": "string", "description": "For export_variants: new local absolute path ending in .csv or .xlsx for the cut lists; the takeoffs go beside it as <name>.takeoff.<ext>."},
                "decimal": {"type": "string", "enum": ["point", "comma"], "description": "For export_variants to .csv: the decimal separator, default point (columns split on ,); comma splits columns on ; as a spreadsheet with a decimal comma expects."},
                "replace_document": {"type": "boolean", "description": "For apply: replace a saved document that no program owns."},
                "name": {"type": "string", "description": "For docs: topic id or example file. For save_variant, use_variant and delete_variant: the variant name."},
                "expected": {"type": "object", "description": "Required for patch/report: existing stamp from the read/apply result. Optional guard for other actions."},
            }, "allOf": [
                {"if": {"properties": {"action": {"const": "patch"}}}, "then": {"required": ["expected", "edits"]}},
                {"if": {"properties": {"action": {"const": "set_params"}}}, "then": {"required": ["expected", "params"]}},
                {"if": {"properties": {"action": {"const": "report"}}}, "then": {"required": ["expected", "section"]}},
                {"if": {"properties": {"action": {"enum": ["save_variant", "use_variant", "delete_variant"]}}}, "then": {"required": ["expected", "name"]}},
                {"if": {"properties": {"action": {"const": "export_variants"}}}, "then": {"required": ["expected", "path"]}}
            ]},
        },
        {
            "name": "inspect",
            "description": "Read the model. status = document, selection and selected_context; summary = overview; \
    operations = catalog of typed CAD operations for model/edit (operation=<name> for one with all its types); \
    query = one page of occurrences/instances/definitions/features/relations/faces/edges (topology rows carry stable reference IDs and exact geometry); \
    detail = one entity (needs kind and entity_id); measure requires expected plus two current face entity IDs with explicit instance paths; mode=minimum measures trimmed faces, supporting_planes measures signed plane clearance along direction, not finite-face overlap. Read-only, rigid placements only, missing exact evidence is never approximated. mass = mass (kg), centre of mass and inertia (kg·mm², named tensor convention, principal moments) of each visible part or of occurrence_ids, and their sum; density from validation_rules.json or the program's material_weight with its citation, never a default; a part without density, solid or rigid placement is not_verified/not_computed with the reason, a hidden part is missing with status hidden, and the assembly is then partial, not complete. workset_create/workset_status = a complete occurrence set for batch. underlay_snap = the endpoint, intersection (two lines crossing, or one ending on another), midpoint and nearest point of the shown underlay lines within radius_mm (default 10, at most 10000) of point_mm (document XY, mm), each with its underlay, layer, distance and drawing_mm (the drawing's own coordinates, e.g. S-JTSK), and line: the length, ends and point count of the line or arc nearest it; with to_mm also measure: from the best snap at point_mm to the best snap at to_mm (endpoint, then intersection, midpoint, nearest; the bare point where none), their delta_mm and distance_mm; an arc's points lie on its chords, within 0.1 mm of the arc; hidden layers give nothing; the underlay is never changed; snaps_ready=false means a just-shown underlay's snap index is still being built and it gave nothing yet, so ask again.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["status", "summary", "operations", "query", "detail", "measure", "mass", "workset_create", "workset_status", "underlay_snap"]},
                "point_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}, "description": "For underlay_snap: the point [x, y] in document millimetres to snap near."}, "to_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}, "description": "For underlay_snap: a second point [x, y] in document millimetres; measures from the snap at point_mm to the snap here."},
                "radius_mm": {"type": "number", "minimum": 0, "maximum": 10000, "description": "For underlay_snap: how far from point_mm to look (default 10)."},
                "underlay_id": {"type": "integer", "minimum": 1, "description": "For underlay_snap: only this underlay; omit for every shown one."},
                "occurrence_ids": {"type": "array", "items": {"type": "integer", "minimum": 1}, "description": "For mass: root occurrences to weigh; omit for every part (hidden ones are listed as missing)."},
                "operation": {"type": "string", "description": "For operations: one operation name, e.g. create_part."}, "faces": face_targets, "mode":{"type":"string","enum":["minimum","supporting_planes"]}, "direction":{"type":"array","minItems":3,"maxItems":3,"items":{"type":"number"},"description":"For measure supporting_planes: the clearance direction. For query kind=faces: the pull direction (definition coordinates) of a draft analysis; each face row gets draft {min/max_angle_degrees, class positive/insufficient/vertical/negative} for planes, cylinders and cones, else not_analyzed with the reason (never estimated)."},
                "min_draft_degrees": {"type": "number", "exclusiveMinimum": 0, "exclusiveMaximum": 90, "description": "With direction: smallest draft counted positive (default 1); a smaller positive draft is insufficient."},
                "kind": {"type": "string", "enum": ["occurrences", "instances", "definitions", "features", "relations", "faces", "edges", "spaces", "clearance_volumes", "joints", "dimensions"], "description": "For query, workset_create and detail (required for detail). spaces, clearance_volumes, joints and dimensions list the product's spaces, keep-clear volumes, joints between derived parts and persistent dimensions (with value and status resolved/ambiguous/lost)."},
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
            }, "allOf": [{"if": {"properties": {"action": {"const": "measure"}}}, "then": {"required": ["expected", "faces", "mode"]}}, {"if": {"properties": {"action": {"const": "measure"}, "mode": {"const": "supporting_planes"}}, "required": ["mode"]}, "then": {"required": ["direction"]}}, {"if": {"properties": {"action": {"const": "underlay_snap"}}}, "then": {"required": ["point_mm"]}}]},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "model",
            "description": "edit_context returns the editable features, parameters and faces of 1 to 8 parts. \
    apply_and_verify applies one typed CAD program ({\"operations\": [...]}, catalog in inspect action=operations), evaluates exact geometry, runs collision (plus validators) and publishes one Undo step. \
    Issues are reported in validation.issues and the edit stays; fix them with a follow-up edit, or pass strict=true to reject instead. One user request = one apply_and_verify. \
    On a document owned by a program the edit detaches the program (result program_detached=true with a warning; strict=true rejects it instead with program_owned_document); prefer program action=apply there. \
    A transform, delete, set_color, mirror or set_feature_parameter of parts the program makes is instead written into the program and the program keeps owning the document: \
    a literal at= is edited or move()/rotate() lines are added, the creating call is removed, color= is set in it, a mirrored() line is added, \
    or a value one param() drives becomes that parameter's override (an extrusion's length otherwise a push_pull() line). \
    A delete or colour of a part made in a loop, by a function of the program or used by another line, or a value written in the source, is refused (planning_rejected) with the line to change.",
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
    import adds a file to the open document as one Undo step: STEP (.step/.stp) and IGES (.iges/.igs) as exact solids, STL as a mesh, DXF as 2D profiles, DWG as the 2D profiles of its DXF conversion (the review's dwg names the file version and every record the DWG reader left out), GLB meshes, SketchUp scenes (.kscene); the result reviews what came in (solids and volume, triangles, profiles and layers) and its diagnostics name what was lost (long lists give the first 20 and a *_total count; the import has happened, do not repeat it). \
    remove_underlay removes an underlay by underlay_id. \
    export writes the visible parts as export_format to path (STEP, IGES, STL, 3MF, GLB, DXF profiles, sheet-metal flat pattern DXF, Kečup Viewer file, fabrication or structural member cut-list CSV, HOMAG MPR, BTLx) plus a .loss.txt report where the format loses information. STEP, IGES, STL, 3MF, GLB, DXF and Viewer files are written only to new paths (an existing file or report is never replaced); manufacturing releases (sheet_metal_dxf, the CSVs, homag_mpr, btlx) wait for the user to confirm in the window; the fabrication CSV, homag_mpr and btlx make the parts of production(tags=…) when the program declares it, shown or hidden. \
    export_drawings writes the project drawings of the visible layers (floor plan cut, longitudinal and cross section, four elevations with overall dimensions) as one vector PDF sheet with a frame and a title block; \
    title_block fields and format given here are kept in the document (an empty string clears a field), scale and format cells are filled in by the sheet. Hidden layers are left out. \
    That sheet is PDF only: it hides back edges by painting over them, which DXF cannot carry. \
    With sheet_id, export_drawings writes that drawing sheet of the document (create_drawing; inspect action=summary lists drawing_sheets) as DXF, or as that DXF converted to an AutoCAD 2018 (AC1032) DWG for a .dwg path: exact views, hidden edges on their own HIDDEN layer, circles the views face as CIRCLE/ARC from the exact circle; dxf_curves counts them and names every exact curve kept as lines, with the reason.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["save", "save_as", "open", "import", "remove_underlay", "export", "export_drawings"]},
                "path": {"type": "string", "description": "Local absolute path. For save_as, open and import (an existing file; its extension picks the format); for export ending in an extension of export_format; for export_drawings a new path ending in .pdf, or .dxf or .dwg with sheet_id (an existing file is never replaced)."},
                "as_underlay": {"type": "boolean", "description": "For import of a .dxf or .dwg: keep the drawing under the model as an underlay to trace and measure (every line, no entity or profile limits; one tag per drawn layer plus one for the whole drawing; never a part, so never in the cut list, collisions or production exports) instead of importing its 2D profiles. The result has underlay_id; inspect action=summary lists underlays with their tags; view action=tag_visibility shows or hides the whole drawing or one layer."},
                "underlay_id": {"type": "integer", "minimum": 1, "description": "For remove_underlay: the underlay to remove with the tags only it held (one Undo step)."},
                "sheet_id": {"type": "integer", "minimum": 1, "description": "For export_drawings: a drawing sheet of the document (inspect action=summary lists drawing_sheets), written as DXF or DWG by the path's extension; format and title_block do not apply."},
                "unit": {"type": "string", "enum": ["mm", "cm", "m", "in", "ft"], "description": "For import: length unit of an STL file (default mm) or of a DXF or DWG file that declares none (required then); a unit the DXF declares wins. STEP and IGES carry their own units."},
                "export_format": {"type": "string", "enum": ["step", "iges", "stl", "3mf", "glb", "dae", "dxf", "sheet_metal_dxf", "viewer", "fabrication_csv", "member_cut_list_csv", "homag_mpr", "btlx"], "description": "For export. Extensions: step .step/.stp, iges .iges/.igs, stl, 3mf, glb, dae (COLLADA 1.4.1 for SketchUp: mm, Z-up, equal parts as one component), dxf and sheet_metal_dxf .dxf (the one sheet-metal part's flat pattern), viewer .ketchup-view, fabrication_csv and member_cut_list_csv .csv, homag_mpr .mpr (file name of exactly 12 letters, digits, - or _), btlx."},
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
    show_view name restores one (tag visibility is one Undo step). tag_visibility name visible hides or shows every part of a tag, or an underlay or one of its layers by their tags (summary lists tags and underlays): \
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
        {
            "name": "run_plugin",
            "description": "Run a local plugin process (ketchup.plugin.v1) against the window's document. \
    Prefer plugin: the absolute path of a plugin folder whose plugin.json alone decides the command line, principal, permissions and declared inputs; \
    send input values in inputs (they reach the plugin by protocol, never as arguments; undeclared, missing, mistyped or together larger than about 128 KiB of JSON: plugin_manifest_invalid, before the user is asked). \
    Otherwise executable with arguments, principal_id, capabilities and allowed_operations (no inputs); plugin excludes those fields. \
    The window first asks the user to allow it, showing absolute paths and SHA-256 (of every file in a plugin folder, and of an interpreter outside it), the command line and the permissions; \
    without that consent the answer is plugin_consent_required and nothing ran. A changed file, another command line or more permissions ask again; other input values do not. \
    The plugin reads the document and sends one typed CAD program, published like model apply_and_verify (stamps, change, validation issues, one Undo step) \
    only while the document is still at the revision the plugin read (stale_document otherwise). \
    A crash, timeout or protocol break returns plugin_failed, a capability or operation outside the grant plugin_refused, each with its cause; the document stays unchanged.",
            "inputSchema": {"type": "object", "properties": {
                "plugin": {"type": "string", "description": "Absolute path of a plugin folder or of its plugin.json."},
                "inputs": {"type": "object", "description": "With plugin: values of the inputs its plugin.json declares, e.g. {\"height_mm\": 35}."},
                "executable": {"type": "string", "description": "Without plugin: absolute path of the plugin executable, e.g. a Python interpreter."},
                "arguments": {"type": "array", "items": {"type": "string"}, "description": "Arguments: at least the absolute path of the plugin script, then plain words (letters, digits and -_.,+/\\). Options that run code given as text, from stdin or by name (e.g. -c, -e, -m, -r, -, --eval, --require, --import), @file arguments and URLs are refused."},
                "principal_id": {"type": "integer", "minimum": 0, "description": "Must equal the principal the plugin declares."},
                "capabilities": {"type": "array", "items": {"type": "string", "enum": ["query.agent-state.v1", "edit.cad-program.v1"]}, "description": "Capabilities granted to the plugin."},
                "allowed_operations": {"type": "array", "items": {"type": "string"}, "description": "Typed operations its program may use, by name (inspect action=operations)."},
                "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 120000, "description": "How long the plugin process may run (default 30000); publishing has its own deadline."},
                "validators": {"type": "array", "items": {"type": "string"}, "description": "Extra validators, e.g. [\"gravity_support\"]."},
                "strict": {"type": "boolean"},
                "expected": {"type": "object", "description": EXPECTED},
            }},
        },
    ])
}
