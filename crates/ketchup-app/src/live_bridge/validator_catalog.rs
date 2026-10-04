//! MCP discovery describes the host's existing checks; it never runs validation.
use serde_json::{Value, json};

pub(super) fn catalog() -> Value {
    let mut validators = ketchup_application::validation::assistant_validator_catalog();
    for entry in &mut validators {
        let id = entry["id"]
            .as_str()
            .expect("document validators have IDs")
            .to_owned();
        entry["kind"] = json!("document");
        entry["run"] =
            json!({"tool":"program", "arguments":{"action":"validate", "validators":[id]}});
        entry["result_path"] = json!(format!("result.validation.document_checks.{id}"));
        entry["requires_program"] = json!(true);
        entry["required_inputs"] = json!(document_inputs(&id));
        entry["limitations"] = json!([
            "Role/material/numeric inputs currently address root parts, not shared component leaves; unchecked instances remain not_evaluated.",
            "Read complete, not_evaluated, assumptions and evaluations; zero issues alone is not a pass."
        ]);
    }
    validators.extend([
        json!({
            "id":"program_geometry", "kind":"program_default",
            "checks":"Native geometry and collisions of visible program parts, declared expectations including minimum distances, and declared joint/machining rules.",
            "required_inputs":["A document owned by a program; declare expectations for intended dimensions/distances."],
            "run":{"tool":"program", "arguments":{"action":"validate"}},
            "result_path":"result.validation", "requires_program":true,
            "limitations":["Not a selectable validators ID: this baseline runs on every program validate call.", "Read geometry, collisions, declared_intent and exact_collisions separately. Accepted source or checked machining rules do not prove exact manufacturing correctness or strength."]
        }),
        json!({
            "id":"motion", "kind":"program_motion",
            "checks":"Continuous native clearance over a named joint's translation or rotation interval.",
            "required_inputs":["A named motion joint in the program.", "motion.name, motion.from and motion.to within declared limits: mm for slide, degrees for rotation."],
            "run":{"tool":"program", "arguments":{"action":"validate", "motion":{"name":"travel", "from":0, "to":100}}},
            "example_arguments":true,
            "result_path":"result.validation.motion", "requires_program":true,
            "limitations":["Use the motion argument, not validators:[\"motion\"]. Replace the example name and interval with the model's declaration.", "Other joints remain at current poses. Unsupported parent frames, unproven contact and exhausted work limits remain incomplete."]
        }),
        json!({
            "id":"assembly_path", "kind":"program",
            "checks":"Declared insertion paths in source order against previously installed parts.",
            "required_inputs":["Named motion joints and assembly_step declarations in intended source order.", "Each inserted rigid part/group appears once; end equals modeled position; motion reference is already installed."],
            "run":{"tool":"program", "arguments":{"action":"validate", "validators":["assembly_path"]}},
            "result_path":"result.validation.assembly_path", "requires_program":true,
            "limitations":["Not an automatic assembly planner, retention check or internal group collision check.", "Parts without steps are already installed; future step members are absent. Missing paths/order and unproven contact remain incomplete."]
        }),
        json!({
            "id":"tool_access", "kind":"program",
            "checks":"Clearance of the entire declared auxiliary tool/holder envelope along its approach.",
            "required_inputs":["An auxiliary tool=True solid and tool_access declaration with envelope, motion and start.", "World-space axis/pivot; end=0 is the modeled working pose."],
            "run":{"tool":"program", "arguments":{"action":"validate", "validators":["tool_access"]}},
            "result_path":"result.validation.tool_access", "requires_program":true,
            "limitations":["Positive clearance required throughout, including working pose. Missing envelopes/paths remain incomplete.", "Does not assess cutting/engagement, cables, hand reach or simultaneous part motion; auxiliary tools do not enter the physical model/BOM."]
        }),
    ]);
    json!({
        "validators":validators,
        "catalog_only":true,
        "scope":"Available host capabilities, not applicability or successful checks of the current document.",
        "documentation":{"tool":"program", "arguments":{"action":"docs", "name":"validation"}},
        "document_edit_route":{
            "tool":"model", "action":"apply_and_verify", "argument":"validators",
            "supported_ids":ketchup_application::validation::ASSISTANT_VALIDATOR_IDS,
            "result_path":"result.validation",
            "warning":"Runs checks while publishing a typed edit, not a read-only validation call. On program-owned documents it detaches the program; prefer program validate."
        },
        "interpretation":{
            "passed":"Only the stated scope was checked. Require complete=true where provided and inspect assumptions and unchecked items.",
            "failed":"A known violation exists; other portions may still be incomplete. Read issues and measured values before changing the model.",
            "incomplete":"Some requested evidence is unavailable; not a pass and not proof the design is impossible.",
            "not_evaluated":"Missing inputs, unsupported geometry or unavailable evidence; supply inputs or report the limitation.",
            "skipped":"Not selected; no conclusion.",
            "not_requested":"No intent/check declared; no conclusion.",
            "accepted_checked_verified":"Intermediate source/rule/geometry states, not overall design safety.",
            "details":"Follow not_evaluated, complete, totals and truncation flags. program report(section=issues, expected=stamp) pages program issues, not every nested validator report.",
            "strength":"Geometric contact is not strength. static_load compares user-declared qualified capacities, not independently verified design data, FEA or certification."
        }
    })
}

fn document_inputs(id: &str) -> Vec<&'static str> {
    match id {
        "group_connectivity" => vec![
            "Group/component membership, physical joint gap declarations and solid contact geometry.",
        ],
        "collision" => vec!["Visible solid bodies and available exact geometry."],
        "assembly_retention" => vec![
            "Declared assembly-retention roles (ketchup.assembly-retention-role.v1), physical pins or fixed assembly joints.",
        ],
        "gravity_support" => vec![
            "Visible bodies, ground/contact geometry and the gravity settings reported by the check.",
        ],
        ketchup_application::validation::DEFLECTION_CHECK => vec![
            "Required validator role with source-frame axes; span/thickness geometry.",
            "ketchup.material.v1 via material=; absent material uses the explicitly reported library default. Design load and limits come from library rules.",
        ],
        "static_load" => vec![
            "Required load/support roles in ketchup.validator-role.v1 with a common case.",
            "physics.gravity_x_m_s2, physics.gravity_y_m_s2, physics.gravity_z_m_s2; physics.mass_kg.occurrence.{occurrence} or physics.applied_load_n.occurrence.{occurrence}; physics.support_capacity_n.occurrence.{occurrence}.",
            "Load classification ketchup.static-load-mode.v1; support classification ketchup.support-capacity.v1 JSON: source, units=N, mode, direction_world, assumptions, additive. Multiple supports require explicit additive load-sharing assumptions.",
            "Program numeric attributes use input:<name>; classification attributes use classification:<dimension>. See validation docs for the exact format.",
        ],
        _ => vec![
            "Matching required_roles in ketchup.validator-role.v1, including axes and association groups where specified; visible geometry and library rule limits.",
        ],
    }
}
