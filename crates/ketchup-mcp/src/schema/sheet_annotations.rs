use serde_json::{Value, json};

pub(super) fn annotations() -> Value {
    let point = json!({"type": "array", "minItems": 2, "maxItems": 2, "items": {"type": "number"}});
    let text = json!({"type": "string", "minLength": 1});
    let tolerance = json!({"oneOf": [
        {"type":"object", "additionalProperties":false, "required":["kind"], "properties":{"kind":{"const":"none"}}},
        {"type":"object", "additionalProperties":false, "required":["kind","deviation"], "properties":{"kind":{"const":"symmetric"},"deviation":{"type":"number","minimum":0}}},
        {"type":"object", "additionalProperties":false, "required":["kind","upper","lower"], "properties":{"kind":{"const":"bilateral"},"upper":{"type":"number","minimum":0},"lower":{"type":"number","minimum":0}}}
    ]});
    let mut variants = Vec::new();
    for (kind, fields, required) in [
        (
            "angular",
            json!({"source_line_ids":{"type":"array","minItems":2,"maxItems":2,"items":text},
            "arc_radius_page_mm":{"type":"number","exclusiveMinimum":0}, "tolerance":tolerance}),
            vec!["source_line_ids", "arc_radius_page_mm"],
        ),
        (
            "radius",
            json!({"source_circle_id":text,"leader_angle_degrees":{"type":"number","minimum":0,"exclusiveMaximum":360},
            "offset_page_mm":{"type":"number","exclusiveMinimum":0},"tolerance":tolerance}),
            vec!["source_circle_id", "leader_angle_degrees", "offset_page_mm"],
        ),
        (
            "diameter",
            json!({"source_circle_id":text,"leader_angle_degrees":{"type":"number","minimum":0,"exclusiveMaximum":360},
            "offset_page_mm":{"type":"number","exclusiveMinimum":0},"tolerance":tolerance}),
            vec!["source_circle_id", "leader_angle_degrees", "offset_page_mm"],
        ),
        (
            "datum",
            json!({"source_line_id":text,"label":text,"offset_page_mm":point}),
            vec!["source_line_id", "label", "offset_page_mm"],
        ),
        (
            "feature_control_frame",
            json!({"source_line_id":text,"offset_page_mm":point,
            "characteristic":{"type":"string","enum":["Straightness","Flatness","Circularity","Cylindricity","ProfileOfLine","ProfileOfSurface","Angularity","Perpendicularity","Parallelism","Position","Concentricity","Symmetry","CircularRunout","TotalRunout"]},
            "tolerance_mm":{"type":"number","exclusiveMinimum":0},"diameter_zone":{"type":"boolean"},
            "material_condition":{"type":"string","enum":["None","MaximumMaterial","LeastMaterial","RegardlessOfFeatureSize"]},
            "datum_labels":{"type":"array","maxItems":3,"items":text}}),
            vec![
                "source_line_id",
                "characteristic",
                "tolerance_mm",
                "offset_page_mm",
            ],
        ),
    ] {
        let mut properties = fields;
        properties["kind"] = json!({"const": kind});
        properties["view"] = text.clone();
        let required: Vec<_> = ["kind", "view"].into_iter().chain(required).collect();
        variants.push(json!({"type":"object", "additionalProperties":false, "required":required, "properties":properties}));
    }
    json!({"type":"array", "minItems":1, "maxItems":768, "items":{"oneOf":variants},
        "description":"For annotate: typed annotations on sheet_id. view and source IDs come from drawing action=sheet; offsets and radii are paper mm. Angular tolerance is degrees; circular tolerance is mm. Datum labels referenced by a frame must exist on the sheet or be added in this batch."})
}
