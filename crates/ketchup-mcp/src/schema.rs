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
    patch requires expected and unique non-overlapping old/new edits, retains overrides and program ownership, and uses the same atomic apply/validation path. Results give source diff, counts and bounded issue/relation previews; report pages return complete model cut_list, hardware, bar_schedule, cutting, quote, cnc_dxf, machining, relations, issues or material_takeoff (BIM quantities of the visible parts). \
    check takes the same source/source_path or expected+edits as apply/patch and answers the same way (change, added/removed, report, exact collisions) without publishing anything: check_only=true, canonical_mutation=false. \
    A rejected program changes nothing and names its line. validate performs a read-only native check of all visible program parts and declared minimum distances, without changing source, selection or Undo history; incomplete checks are not passes. \
    docs returns the library index; with name, concise signatures/rules/example; detail=implementation returns helper code explicitly. Named example files return runnable source. \
    from_underlay with expected and name writes what an imported drawing draws into the program as data in document mm, so the model follows the drawing without copying coordinates: point_mm the outline NAME and holes NAME_HOLES of the innermost closed region around that point (layers, tolerance_mm as underlay_regions), label those of the region holding that text (no lookup of the text or region first), object_id those of one closed object (a hatch or closed polyline), or block the sampled linework as a native shared component NAME and its affine instances, including every array copy, without extrusion. NAME and each hole of NAME_HOLES are a list of corner points, or, when the outline has round edges, of its sides as named segments [\"e1\", (x, y), (x, y)] with arcs as [\"e2\", start, end, {\"through\": (x, y)}], which profile= reads as true arcs; so their form can change when an edit of the drawing reveals an arc. NAME_POINTS and NAME_HOLES_POINTS always give the corners as points (a round edge as its chords): code that walks the outline as points reads those. Texts, attributes and fills remain in the underlay; source comments keep placement handles and attributes. Region/object conversion also publishes a native NAME_sketch component with a closed outline profile and separate closed hole profiles; with thickness_mm also the solid NAME with the holes cut through, so a solid from a drawn outline is one call. Without thickness_mm there is no solid; the data can be used as extrude(\"floor\", profile=NAME, distance=…) then cut(…, profile=hole, through=True) per hole. The section's first comment states the area, perimeter and holes found: check them in source_diff and Undo if it is not the intended region. Generated sections retain a binding to the document underlay, with the selection and explicit closures in drawing coordinates. Changes to that underlay's geometry or placement regenerate the section and downstream model in the same Undo step; missing or ambiguous selections reject the edit without stale geometry. This does not watch the external DWG file. Calling again with the same name changes the selection. Applied like apply: one Undo step; without a program it starts one. \
    \"This face\"/\"this edge\": inspect action=status -> selected_context.program names the user's pick in program terms.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["read", "apply", "patch", "check", "set_params", "save_variant", "use_variant", "delete_variant", "export_variants", "report", "docs", "validate", "from_underlay"]},
                "mode": {"type": "string", "enum": ["source", "selection", "outline", "full"], "description": "For read: default source. Selection lines are context, not a dependency closure. outline lists defs, params and comment sections with line numbers."},
                "lines": {"type": "array", "items": {"type": "integer", "minimum": 1}, "minItems": 2, "maxItems": 2, "description": "For read: inclusive 1-based line range [first, last] of the source (at most 400 lines per answer)."},
                "search": {"type": "string", "minLength": 1, "description": "For read: the source lines containing this text, ignoring case."},
                "part": {"type": "string", "description": "For read: the source lines that define this program part, with two neighbours."},
                "params": {"type": "object", "additionalProperties": {"type": "number"}, "description": "For set_params: new values of declared parameters by name, e.g. {\"width\": 900}."},
                "validators": {"type": "array", "items": {"type": "string"}, "description": "For validate: optional existing document validators, e.g. [\"gravity_support\"], with results in validation.document_checks. Program validator \"assembly_path\" checks assembly_step declarations in source order; results in validation.assembly_path. Missing order/paths and unproven contact cannot pass. Program validator \"tool_access\" checks declared auxiliary envelope approaches against visible solids; results in validation.tool_access, no physical tool part is created. Missing envelopes/paths remain incomplete. Does not detach the program."},
                "motion": {"type": "object", "required": ["name", "from", "to"], "additionalProperties": false, "properties": {"name": {"type": "string"}, "from": {"type": "number"}, "to": {"type": "number"}}, "description": "For validate: check a named program motion over the entire interval (mm for slide, degrees for rotation), within its declared limits. Other joints stay at their current poses. Native interval proof; contact, work limits, unsupported bounds or driven parent frames remain incomplete. Results in validation.motion; no document mutation."},
                "edits": {"type": "array", "minItems": 1, "maxItems": 100, "items": {"type": "object", "required": ["old", "new"], "additionalProperties": false, "properties": {"old": {"type": "string", "minLength": 1}, "new": {"type": "string"}}}, "description": "For patch and check: unique exact text replacements, all matched against the original source. expected is required; stale/ambiguous/overlapping edits change nothing."},
                "section": {"type": "string", "enum": ["cut_list", "hardware", "bar_schedule", "cutting", "quote", "cnc_dxf", "machining", "relations", "issues", "material_takeoff", "selection_takeoff", "joints", "loads", "members"], "description": "For report: required with expected. Cut list has one row per made part plus group/group_count; hardware one row per bought item (a joint's fastener, or what its rating is bought as: a catalogued connector and the nails it counts, or the parts with a designation attribute: item = designation, count, parts, attributes); bar_schedule one row per position of bars cut (and bent) to length, e.g. rebar (the parts with a cut_length_mm attribute: position, diameter_mm, grade, shape, cut_length_mm, count, total_length_m, mass_kg derived, parts); neither is in the cut list; cutting one row per piece of the made parts whose material has stock() (bar_stock/sheet_stock), laid on bars of stock lengths or guillotine-cut sheets with kerf and trim: material, kind lengths/sheets, section_mm, stock (sizes and counts used), stock_no, stock_mm, used_percent, offcut_mm, piece (part, at_mm, size_mm, turned), or unplaced (part, reason) for a piece no stock holds; a heuristic, not a proven optimum; quote prices what the lists count at the program's price()/price_list() unit prices (there are no built-in prices): one row per priced item (kind material/hardware/bars/operation, item, quantity, unit m3/m2/m/piece/kg/operation, unit_price, amount, currency, basis stock or blanks for materials, source), then a total row (total, currency, unpriced_count), then unpriced (kind, item, reason) for whatever is counted without a usable price and is not in the total, then unused_price for prices matching nothing; empty without prices; cnc_dxf one AutoCAD R12 DXF per made flat part in the production scope (fabrication role manufactured item, an extrusion; a box seen on its largest face) as rows part, file, thickness_mm, size_mm, layers, chunk/chunks and dxf text (join a file's chunks in order), layers CONTOUR, CUTOUT, DRILL_D<d>_Z<depth>[_BOTTOM], HDRILL_D<d>_Z<depth>_H<height>, POCKET_Z<depth>[_BOTTOM], PART_NAME, then not_drawn (part, reason) for a part whose shape a drawing on its face cannot carry; machining one operation per row. Reassemble by group/part. Cut list, hardware, bar schedule, machining and takeoffs count only the parts of production(tags=…) when the program declares it, shown or hidden. Without it material_takeoff counts only visible parts (hidden layers excluded): rows by category/material/cross-section with count, length_m, area_m2, volume_m3 and volume_basis (exact solid or blank), then one material_total per material. selection_takeoff is the same for the selected visible parts only: the summary of one wall or floor. joints lists every bearing joint, and every joint holding a member down, with its published rating (load_n, tension_n, basis, source, density_kg_m3: the least timber density it holds for, service_classes: the EN 1995 service classes it holds in; a catalogued connector that does not fit the members has none), its pull (tension, checked against tension_n), its utilization on that basis (allowable vs characteristic load, characteristic as kmod Rk/1.3, design vs design load) and status pass/fail, or not_verified and what is missing; never a pass without a rating and a load. loads lists every load_path member with characteristic loads by kind (N), patches along it and reactions per support; missing names absent inputs. members lists the EN 1995-1-1 check of every load_path member of a timber_design() material: role, section, checks with utilization, combination and place, the governing utilization and status pass/fail/not_verified (not_verified whenever an input is missing); gravity loads only, wind and horizontal stability are not checked. Model-derived, not a prose estimate."},
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
                "name": {"type": "string", "description": "For docs: topic id or example file. For save_variant, use_variant and delete_variant: the variant name. For from_underlay: the program name of the data, e.g. ROOM_1_02."},
                "expected": {"type": "object", "description": "Required for patch/report/from_underlay: existing stamp from the read/apply result. Optional guard for other actions."},
                "underlay_id": {"type": "integer", "minimum": 1, "description": "For from_underlay: the underlay (may be omitted when the document has one)."},
                "point_mm": {"type": "array", "items": {"type": "number"}, "minItems": 2, "maxItems": 2, "description": "For from_underlay: a document point [x, y] inside the closed region to write."},
                "label": {"type": "string", "minLength": 1, "description": "For from_underlay instead of point_mm: a text written inside the region, e.g. \"1.02 WC\" (any case; line breaks count as spaces). A text equal to it wins over texts containing it; the text must lie wholly inside one region. Several regions or none are refused with what was found. The binding keeps the label, so the outline follows the room when the drawing changes."},
                "thickness_mm": {"type": "number", "exclusiveMinimum": 0, "description": "For from_underlay with point_mm, label or object_id: also extrude the outline this far as the solid NAME (from z = 0 up) with every hole cut through, in the same Undo step; the generated section keeps it when the drawing changes."},
                "layers": {"type": "array", "items": {"type": "string"}, "description": "For from_underlay with point_mm or label: the layers whose lines close the region; default the shown ones."},
                "tolerance_mm": {"type": "number", "exclusiveMinimum": 0, "description": "For from_underlay with point_mm or label: line ends closer than this join, as underlay_regions."},
                "closing_segments_mm": {"type": "array", "maxItems": 100, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}}, "description": "For from_underlay with point_mm or label: explicit helper boundaries [[x,y],[x,y]] in document mm, e.g. across a doorway. Never inferred automatically; the underlay is unchanged."},
                "object_id": {"type": "integer", "minimum": 0, "description": "For from_underlay: an object id from inspect underlay_objects whose closed outline to write. Without a source handle the id belongs only to the current drawing data: placement edits remain linked, but replacing the data is rejected. Unlink the generated section before replacing such data, then select the intended object again; its position in the object list is not persistent identity."},
                "block": {"type": "string", "description": "For from_underlay: the block (any case) whose sampled linework to convert to a shared 2D component and affine instances."},
            }, "allOf": [
                {"if": {"properties": {"action": {"const": "patch"}}}, "then": {"required": ["expected", "edits"]}},
                {"if": {"properties": {"action": {"const": "set_params"}}}, "then": {"required": ["expected", "params"]}},
                {"if": {"properties": {"action": {"const": "report"}}}, "then": {"required": ["expected", "section"]}},
                {"if": {"properties": {"action": {"enum": ["save_variant", "use_variant", "delete_variant"]}}}, "then": {"required": ["expected", "name"]}},
                {"if": {"properties": {"action": {"const": "export_variants"}}}, "then": {"required": ["expected", "path"]}},
                {"if": {"properties": {"action": {"const": "from_underlay"}}}, "then": {"required": ["expected", "name"]}}
            ]},
        },
        {
            "name": "inspect",
            "description": "Read the model. status = document, selection, selected_context and picked_annotation (the annotation clicked in the window); summary = overview; \
    operations = catalog of typed CAD operations for model/edit (operation=<name> for one with all its types); \
    query = one page of occurrences/instances/definitions/features/relations/faces/edges (topology rows carry stable reference IDs and exact geometry); \
    detail = one entity (needs kind and entity_id); measure requires expected plus two current face entity IDs with explicit instance paths; mode=minimum measures trimmed faces, supporting_planes measures signed plane clearance along direction, not finite-face overlap. Read-only, rigid placements only, missing exact evidence is never approximated. mass = mass (kg), centre of mass and inertia (kg·mm², named tensor convention, principal moments) of each visible part or of occurrence_ids, and their sum; density from validation_rules.json or the program's material_weight with its citation, never a default; a part without density, solid or rigid placement is not_verified/not_computed with the reason, a hidden part is missing with status hidden, and the assembly is then partial, not complete. workset_create/workset_status = a complete occurrence set for batch. underlay_snap = the endpoint, intersection (two lines crossing, or one ending on another), midpoint and nearest point of the shown underlay lines within radius_mm (default 10, at most 10000) of point_mm (document XY, mm), each with its underlay, layer, distance and drawing_mm (the drawing's own coordinates, e.g. S-JTSK), and line: the length, ends and point count of the line or arc nearest it; with to_mm also measure: from the best snap at point_mm to the best snap at to_mm (endpoint, then intersection, midpoint, nearest; the bare point where none), their delta_mm and distance_mm; an arc's points lie on its chords, within 0.1 mm of the arc; hidden layers give nothing; the underlay is never changed; snaps_ready=false means a just-shown underlay's snap index is still being built and it gave nothing yet, so ask again. underlay_objects = what an imported DXF/DWG underlay is made of as objects, not lines: objects_total; without filters on the first page also kinds {DXF kind: [top level, all]}, blocks (block, top_level, all; most placed first), layers {name: objects}; and one page (limit, offset, next_offset) of the objects that match every filter given: object_kind (LINE, LWPOLYLINE, MTEXT, INSERT, DIMENSION, HATCH, …), layer, block (part of the block name an insert places), text (part of a text drawn by it or below it, or an attribute value), within_mm (document box [[x,y],[x,y]] its bounds meet), parent_id (the children of an insert or dimension); without parent_id only top-level objects unless top_level=false. A row: id, kind, layer, handle (hex, as in the drawing), parent, children, bounds_mm, text; a single text also has text_anchor_mm (insertion point, possibly outside the visible letters) and text_point_mm (centre between laid-out letter centres, a candidate query point, not proof of a room); and an insert's block, attributes, insertion_mm, rotation_deg, scale and mirrored, a dimension's measurement or a hatch's pattern/area_m2. object_id gives one object in full: child_ids, drawing_bounds_mm, an insert's drawing_insertion_mm and copies; a dimension's dimension_type, measurement (as the drawing records it), text_override and points; a hatch's solid and rings_mm; its own texts and polylines_mm with length_mm (at most 4000 points, points_truncated); touching: the ids of the top-level objects whose lines join its lines (a corner or line end within 1 mm of the other's line; a dimension's definition points on the lines it measures; hatches left out), touching_total. Points are document mm; drawing_* the drawing's own coordinates. An underlay imported before objects were kept has none (objects_total 0): import it again. Read-only. underlay_regions = the closed regions (rooms, parcels, cut-outs alike) the lines of the shown layers, or of layers (names), enclose: lines are split where they cross or one ends on another within tolerance_mm (default 1), lines with a loose end and hatch and dimension lines are left out, so a door gap leaves a room open unless the door's own lines close it (choose layers accordingly); rows (largest first, or with point_mm the regions holding that point, innermost first, with outline) give region, area_m2, perimeter_mm, bounds_mm, corners, holes and the texts of the shown layers whose rendered letter centres all lie inside (innermost region), outline_mm/holes_mm with include_outline; ambiguous_texts lists texts crossing regions with candidate_regions and partly_outside. These are geometric regions, not classified rooms; a label or successful extrusion does not prove a room boundary. Check the chosen layers, outline and drawing dimensions/legend before modeling; min_area_m2 (default 0.1) drops slivers, within_mm limits to lines meeting a document box; limit/offset page. underlay_parallel = the pairs of lines of the shown or named layers within angle_deg (default 1) of parallel, min_gap_mm to max_gap_mm apart (default 1 to 1000) and alongside each other at least min_overlap_mm (default 100): gaps (the most common distances with pairs and alongside_m, e.g. wall thicknesses) and pairs (gap_mm, alongside_mm, both lines, each with the object_id that draws it). Region and pair numbers hold only for the same parameters.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["status", "summary", "operations", "query", "detail", "measure", "mass", "workset_create", "workset_status", "underlay_snap", "underlay_objects", "underlay_regions", "underlay_parallel"]},
                "layers": {"type": "array", "items": {"type": "string"}, "description": "For underlay_regions and underlay_parallel: drawing layer names to use; omit for the shown layers."}, "min_area_m2": {"type": "number", "description": "For underlay_regions: smallest region kept (default 0.1)."}, "tolerance_mm": {"type": "number", "exclusiveMinimum": 0, "maximum": 1000, "description": "For underlay_regions: line ends and crossings closer than this join (default 1)."}, "include_outline": {"type": "boolean", "description": "For underlay_regions: give outline_mm and holes_mm (default only with point_mm)."}, "whole": {"type": "boolean", "description": "For underlay_regions: whole figures instead of regions, each the outer edge of lines joined together whatever lines split it inside, closed figures inside it its holes: the views of a part on a sheet, each its silhouette (default false)."},
                "min_gap_mm": {"type": "number", "minimum": 0}, "max_gap_mm": {"type": "number", "minimum": 0, "maximum": 10000}, "min_overlap_mm": {"type": "number", "minimum": 0}, "angle_deg": {"type": "number", "minimum": 0, "maximum": 45, "description": "For underlay_parallel: how far from parallel (default 1)."},
                "object_id": {"type": "integer", "minimum": 0, "description": "For underlay_objects: one object in full."}, "object_kind": {"type": "string", "description": "For underlay_objects: DXF kind, e.g. INSERT, HATCH, MTEXT, LWPOLYLINE."}, "layer": {"type": "string", "description": "For underlay_objects: drawing layer name (any case)."}, "block": {"type": "string", "description": "For underlay_objects: part of the block name an insert places."}, "text": {"type": "string", "description": "For underlay_objects: part of a text or attribute value, any case."},
                "within_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}, "description": "For underlay_objects: document box [[x,y],[x,y]] the object's bounds meet; for underlay_regions and underlay_parallel: only lines meeting it."}, "parent_id": {"type": "integer", "minimum": 0, "description": "For underlay_objects: list the children of this insert or dimension."}, "top_level": {"type": "boolean", "description": "For underlay_objects without parent_id: false lists objects inside blocks too (default true)."}, "offset": {"type": "integer", "minimum": 0, "description": "For underlay_objects: first row of the page (next_offset of the previous)."},
                "point_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}, "description": "For underlay_snap: the point [x, y] in document millimetres to snap near. For underlay_regions: only the regions holding it."}, "to_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}, "description": "For underlay_snap: a second point [x, y] in document millimetres; measures from the snap at point_mm to the snap here."},
                "closing_segments_mm": {"type": "array", "maxItems": 100, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}}, "description": "For underlay_regions: explicit helper boundaries [[x,y],[x,y]] in document mm to close selected gaps. Echoed in the answer; never inferred automatically and never changes the underlay. Pass the same segments to from_underlay after verifying the outline."},
                "radius_mm": {"type": "number", "minimum": 0, "maximum": 10000, "description": "For underlay_snap: how far from point_mm to look (default 10)."},
                "underlay_id": {"type": "integer", "minimum": 1, "description": "For underlay_snap: only this underlay; omit for every shown one. For underlay_objects: the underlay (may be omitted when the document has one)."},
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
    fit_underlay places underlay_id so that from_mm, two points of its lines where they are drawn now (e.g. inspect action=underlay_snap point_mm), land on the model points to_mm (document XY, mm), as one Undo step saved with the file: it turns and moves the underlay so the first lands exactly and the second on the line from it; scale=true also scales it so the second lands too (default false; the result's length_mismatch_mm says how far the second misses without it). Two points closer than 0.001 mm are refused (invalid_params). Drawing, snapping and underlay_snap then use the placed lines. \
    export writes the visible parts as export_format to path (STEP, IGES, STL, 3MF, GLB, DXF or DWG plan: profiles plus the program's annotations as TEXT and DIMENSION on the layer of their tag at drawing_scale, sheet-metal flat pattern DXF, Kečup Viewer file, fabrication or structural member cut-list CSV, HOMAG MPR, BTLx) plus a .loss.txt report where the format loses information. STEP, IGES, STL, 3MF, GLB, DXF, DWG and Viewer files are written only to new paths (an existing file or report is never replaced); manufacturing releases (sheet_metal_dxf, the CSVs, homag_mpr, btlx) wait for the user to confirm in the window; the fabrication CSV, homag_mpr and btlx make the parts of production(tags=…) when the program declares it, shown or hidden. \
    export_drawings writes the project drawings of the visible layers as a numbered set of sheets at one scale, each with a frame and a title block saying what it shows and its number (n / N): a floor plan of each storey (a level holding floor slabs over most of the footprint), a sheet of the longitudinal and cross section and a sheet of the four elevations with overall dimensions; then a shop drawing of each build-up of library buildup() (parts with the attribute SHOP_DRAWING = \\\"drawing:sheet\\\"), at its own scale: the build-up seen square on, its members and what lies between them in one view and each sheet layer covering them in a view of its own beside or below, each piece with its position number, running dimensions of the upright members' edges (between them the openings) and of the plates, sills and headers, and a table of the positions with material, size and count; with library window()/door() openings each plan marks them by type (W1, D1, …) and a last sheet lists every mark with its designation and count, as the hardware report counts them. A .pdf path gets one vector PDF of all sheets; a .dxf or .dwg path gets a file per sheet beside it, numbered (plan-01.dxf, plan-02.dxf, …) and listed in plan.dxf.sheets.txt, of its visible lines only (hidden lines removed, cut outlines on layer CUT, texts on TEXT, paper mm), the DWG as AutoCAD 2018. \
    title_block fields and format given here are kept in the document (an empty string clears a field; a value too long for its cell even in 1.6 mm lines refuses the export with title_value_does_not_fit, never cut short); drawing, when given, names the set before each sheet's own title; scale, format and sheet cells are filled in by each sheet. Hidden layers are left out. printed_as_question_mark names every character of the sheets' texts their typeface lacks, which the PDF prints as \"?\". \
    Each plan draws the visible annotations of the program on its storey (text, dim with its measured value, leader, polyline); plan_only=true writes the plans alone, x to the right, and drawing_scale=N draws every sheet but the shop drawings at 1:N (each on the smallest format that holds it unless format is given; drawings_do_not_fit when none does); the result lists the sheets with their files, views and the annotations drawn. \
    With sheet_id, export_drawings writes that drawing sheet of the document (create_drawing; inspect action=summary lists drawing_sheets) as DXF, or as that DXF converted to an AutoCAD 2018 (AC1032) DWG for a .dwg path: exact views, hidden edges on their own HIDDEN layer, circles the views face as CIRCLE/ARC from the exact circle; dxf_curves counts them and names every exact curve kept as lines, with the reason; on a sheet of several parts, drawn where they lie, its top view draws the shown program annotations (texts, dimensions with the measured value, leaders, lines), and annotations counts them, those of a one-part sheet drawn in the part's own frame (without_plan_view), the dimensions straight up (outside_plan) and by name those that would leave the top view's cell (off_sheet).",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["save", "save_as", "open", "import", "remove_underlay", "fit_underlay", "export", "export_drawings"]},
                "path": {"type": "string", "description": "Local absolute path. For save_as, open and import (an existing file; its extension picks the format); for export ending in an extension of export_format; for export_drawings a new path ending in .pdf, .dxf or .dwg (an existing file is never replaced)."},
                "as_underlay": {"type": "boolean", "description": "For import of a .dxf or .dwg: keep the drawing under the model as an underlay to trace and measure (every line, no entity or profile limits; one tag per drawn layer plus one for the whole drawing; never a part, so never in the cut list, collisions or production exports) instead of importing its 2D profiles. The result has underlay_id; inspect action=summary lists underlays with their tags; view action=tag_visibility shows or hides the whole drawing or one layer."},
                "underlay_id": {"type": "integer", "minimum": 1, "description": "For remove_underlay: the underlay to remove with the tags only it held (one Undo step). For fit_underlay: the underlay to place."},
                "from_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}, "description": "For fit_underlay: two points [[x, y], [x, y]] of the underlay's lines where they are drawn now, in document millimetres."},
                "to_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}, "description": "For fit_underlay: the two model points [[x, y], [x, y]] they land on, in document millimetres."},
                "scale": {"type": "boolean", "description": "For fit_underlay: also scale the underlay so both points land (default false: turn and move only)."},
                "sheet_id": {"type": "integer", "minimum": 1, "description": "For export_drawings: a drawing sheet of the document (inspect action=summary lists drawing_sheets), written as DXF or DWG by the path's extension; format and title_block do not apply."},
                "unit": {"type": "string", "enum": ["mm", "cm", "m", "in", "ft"], "description": "For import: length unit of an STL file (default mm) or of a DXF or DWG file that declares none (required then); a unit the DXF declares wins. STEP and IGES carry their own units."},
                "export_format": {"type": "string", "enum": ["step", "iges", "stl", "3mf", "glb", "dae", "dxf", "dwg", "sheet_metal_dxf", "viewer", "fabrication_csv", "member_cut_list_csv", "homag_mpr", "btlx"], "description": "For export. Extensions: step .step/.stp, iges .iges/.igs, stl, 3mf, glb, dae (COLLADA 1.4.1 for SketchUp: mm, Z-up, equal parts as one component), dxf and sheet_metal_dxf .dxf (the one sheet-metal part's flat pattern), dwg .dwg (the dxf plan as AutoCAD 2018 DWG), viewer .ketchup-view, fabrication_csv and member_cut_list_csv .csv, homag_mpr .mpr (file name of exactly 12 letters, digits, - or _), btlx."},
                "drawing_scale": {"type": "number", "minimum": 1, "maximum": 100000, "description": "For export dxf or dwg: N of the scale 1:N the annotations are drawn at (default 100): a text height_mm on paper is written N times taller in the file, which is in model millimetres. For export_drawings: the whole number N of the scale 1:N the sheet is drawn at (default: the sheet picks it)."},
                "plan_only": {"type": "boolean", "description": "For export_drawings: only the plans with their annotations, x to the right, without the sections and elevations (default false)."},
                "format": {"type": "string", "enum": ["auto", "A3", "A2", "A1", "A0"], "description": "For export_drawings: sheet format of every sheet; auto gives each sheet the smallest that holds its views at the set's scale, 1:50 or finer. Omit to keep the document's format."},
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
            "name": "drawing",
            "description": "2D CAD on a DXF/DWG drawing imported as an underlay (file action=import as_underlay=true): changes the drawing itself, not the model. Objects are the rows of inspect action=underlay_objects, by id; only top-level ones are edited: an insert, dimension or hatch is one object with everything it draws. \
    remove takes object_ids, or instead every top-level object that meets the document box within_mm (as underlay_objects within_mm finds them; the answer lists the ids), out of the drawing as one Undo step (Undo brings them back): their lines, texts and fills leave the view, snapping, underlay_objects, underlay_regions, underlay_parallel and from_underlay; every other object keeps its id, so ids from one underlay_objects answer stay valid. The imported file is never changed; Save keeps the edits in the document. An id inside an insert or dimension is refused with remove_instead, the top-level object holding it; an id already removed is entity_not_found. A drawing imported before Kečup kept objects has none: import it again. \
    draw adds shapes on the existing layer named layer (underlay_objects lists the layers), as new objects of the drawing, one Undo step; the answer gives their object_ids. shapes are in document mm: {shape: line, points_mm: [[x,y],[x,y]]}, {shape: polyline, points_mm, closed}, {shape: arc, center_mm, radius_mm, start_deg, end_deg} counterclockwise, {shape: circle, center_mm, radius_mm}, {shape: text, anchor_mm, height_mm, rotation_deg, text}. A bad shape is refused with shape_index; an unknown layer with layer. \
    move moves object_ids (drawn ones too): scaled by scale and turned counterclockwise by rotate_deg about about_mm (default [0,0]), mirrored across the line through the two points mirror_line_mm, then moved by by_mm; give at least one of them. copy=true keeps the objects and adds such copies (new_object_ids in the answer; a copied insert places its block again). One Undo step; ids stay. \
    trim, extend and offset edit one straight line object_id (a LINE or two-point polyline; anything else is refused with object_id): trim cuts away its piece around at_mm up to the nearest lines of the drawing crossing it on either side (or to its end); extend takes its end nearer at_mm on to the first line ahead of it; offset draws a parallel line on the side of at_mm, distance_mm away or through at_mm. Only lines of shown layers count for trim and extend, not those of hatches, fills or dimensions. The line keeps its object_id and, written back to the DXF/DWG, its entity with its handle, colour, line type, lineweight, width and data: trim and extend give it new ends, and a second piece of a trim and an offset are copies of it (new_object_ids in the answer); nothing crossing or ahead is refused. One Undo step each. \
    export writes the drawing with these changes to the DXF file path (or DWG, for a path ending in .dwg: the same DXF saved as DWG): the file it was imported from (source, or the one imported in this window; a DWG is converted) is read again and written as it was, its header, layers, styles, blocks, dimensions, hatches and unchanged objects kept, the removed objects left out, the moved ones where they lie and the drawn and copied ones added after them as LINE, LWPOLYLINE, ARC, CIRCLE and TEXT; importing the written file gives the edited drawing. A source that is not the imported drawing is refused; a moved hatch or other object that is not mapped yet is refused with its id. The imported file itself is never changed. \
    In the window: a click on a line of the drawing picks its object, Shift+click picks more or drops one, Delete removes them, Move and Rotate move them (Ctrl copies); a double-click selects the whole drawing, which Move and Rotate then place; Tools > Export drawing to DXF and to DWG do export. \
    extrude pulls a closed area of the drawing up height_mm into a solid standing on the ground, its holes cut through, one Undo step: the innermost closed region of the shown layers around at_mm (document mm), or the closed polyline or hatch object_id. Where a program owns the document, or the document holds nothing yet, the solid is written into the program as program action=from_underlay with thickness_mm writes it (data AREA_n, bound to the drawing: it follows when the drawing is edited; bound_to_drawing=true; an edit by remove, draw, move, trim, extend or offset that changes such an area answers areas_changed [{name, was, now}], its area, perimeter and holes before and after, and one that leaves it no closed outline is refused naming it: to keep its present outline, unlink first with Tools > Unlink pulled areas from the drawing, or program action=patch removing its \"# underlay binding\" line); otherwise it is added as a part of its own named AREA_n. Nothing closed there is refused with at_mm. In the window: Tools > Pull drawing area up, the height typed, a click in the area. \
    view sets the drawing up in space, one Undo step: plane=top lays it in the ground plane (plane_at_mm its height); front (seen from -Y: drawing X along world X, drawing Y up), back (from +Y: X along -X), left (from -X: X along -Y) and right (from +X: X along +Y) stand it on a vertical plane plane_at_mm along the axis it is seen along. Align a view first, while it lies in the plan, with fit_underlay or move so that its document point [u, v] is where the plan's X (or Y) and the height are: a view's document point [u, v] lies at origin_world_mm + u * drawing_x_world + v * drawing_y_world. A drawing set up as a view is drawn standing; the window's pick, snap and edit tools reach only drawings lying in the plan. \
    solid_from_views builds the solid that 2 to 8 drawings set up as views seen along at least two different axes show together (a plan and a front view, or front and side, from one file imported twice or from several files): views gives per drawing underlay_id and the area it shows (at_mm, a point inside it in that drawing's document mm: the whole figure drawn around it, the view's silhouette with the edges inside it ignored, unless whole=false; or label, or object_id; layers and closing_segments_mm as for underlay_regions, layers to leave out dimensions, hidden lines and the frame). Each area is written into the program as data NAME_V1, NAME_V2, … with its frame (bound to the drawing: it follows edits, a new placement or view of the drawing), pulled along its line of sight through the box of all views, its holes cut through, and the pulls intersected into the part NAME (name, default SOLID_n); one Undo step. Needs a document a program owns or an empty one.",
            "inputSchema": {"type": "object", "required": ["action"], "properties": {
                "action": {"type": "string", "enum": ["remove", "draw", "move", "trim", "extend", "offset", "export", "extrude", "view", "solid_from_views"]},
                "plane": {"type": "string", "enum": ["top", "front", "back", "left", "right"], "description": "For view: the plane the drawing stands on."},
                "plane_at_mm": {"type": "number", "description": "For view: where the plane lies along the axis it is seen along (a plan's height), default 0."},
                "name": {"type": "string", "minLength": 1, "description": "For solid_from_views: the program name of the solid, letters, digits and _."},
                "views": {"type": "array", "minItems": 2, "maxItems": 8, "items": {"type": "object", "required": ["underlay_id"], "properties": {
                    "underlay_id": {"type": "integer", "minimum": 1},
                    "at_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}},
                    "label": {"type": "string", "minLength": 1},
                    "object_id": {"type": "integer", "minimum": 0},
                    "layers": {"type": "array", "items": {"type": "string"}},
                    "closing_segments_mm": {"type": "array", "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}}},
                    "whole": {"type": "boolean", "description": "With at_mm: true (default) the whole figure drawn around the point, its outer edge, edges seen within the view ignored, closed figures inside it holes; false the innermost region around it."},
                }}, "description": "For solid_from_views: the drawings set up as views and the area each shows."},
                "underlay_id": {"type": "integer", "minimum": 1, "description": "The underlay (may be omitted when the document has one)."},
                "object_ids": {"type": "array", "minItems": 1, "maxItems": 100000, "items": {"type": "integer", "minimum": 0}, "description": "For remove and move: top-level object ids from inspect action=underlay_objects or from draw. remove takes this or within_mm."},
                "within_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}, "description": "For remove: [[x_min, y_min], [x_max, y_max]] in document mm; every top-level object meeting it goes."},
                "layer": {"type": "string", "minLength": 1, "description": "For draw: the drawing's layer the shapes go on."},
                "shapes": {"type": "array", "minItems": 1, "maxItems": 10000, "items": {"type": "object", "required": ["shape"], "properties": {
                    "shape": {"type": "string", "enum": ["line", "polyline", "arc", "circle", "text"]},
                    "points_mm": {"type": "array", "minItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}},
                    "closed": {"type": "boolean"},
                    "center_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}},
                    "radius_mm": {"type": "number", "exclusiveMinimum": 0},
                    "start_deg": {"type": "number"},
                    "end_deg": {"type": "number"},
                    "anchor_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}},
                    "height_mm": {"type": "number", "exclusiveMinimum": 0},
                    "rotation_deg": {"type": "number"},
                    "text": {"type": "string", "minLength": 1},
                }}, "description": "For draw: the shapes, in document mm."},
                "by_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}},
                "rotate_deg": {"type": "number", "description": "For move: counterclockwise turn about about_mm."},
                "scale": {"type": "number", "exclusiveMinimum": 0, "description": "For move: uniform scale about about_mm."},
                "about_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}},
                "mirror_line_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}}, "description": "For move: two points of the mirror line, document mm."},
                "copy": {"type": "boolean", "description": "For move: keep the objects and add moved copies."},
                "object_id": {"type": "integer", "minimum": 0, "description": "For trim, extend and offset: the straight line (a LINE or a two-point polyline). For extrude: the closed polyline or hatch to pull up, instead of at_mm."},
                "height_mm": {"type": "number", "exclusiveMinimum": 0, "description": "For extrude: how high the area is pulled up, document mm."},
                "at_mm": {"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}, "description": "For trim: a point on the piece to cut away; extend: near the end to extend; offset: the side, and the point the new line passes through without distance_mm; extrude: a point inside the area. Document mm."},
                "distance_mm": {"type": "number", "exclusiveMinimum": 0, "description": "For offset: how far the new line lies from the old one, document mm."},
                "path": {"type": "string", "minLength": 1, "description": "For export: the file to write, a local absolute path ending in .dxf or .dwg."},
                "source": {"type": "string", "minLength": 1, "description": "For export: the DXF or DWG file the drawing was imported from, when it was not imported in this window."},
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
                "arguments": {"type": "array", "items": {"type": "string"}, "description": "Arguments: the absolute path of the plugin script first (only harmless interpreter options such as python -B or -u may precede it), then plain words (letters, digits and -_.,+/\\) for the script. Other options before the script, @file arguments, URLs and an executable starting with #! are refused."},
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
