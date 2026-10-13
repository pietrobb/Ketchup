//! `program action=save_variant/use_variant/delete_variant`: named sets of all
//! parameter values kept with the program. Using one re-applies the program
//! like `set_params`, so it is one Undo step however many values change.
#[cfg(test)]
#[path = "program_variants_tests.rs"]
mod tests;

use super::program_access::invalid;
use super::*;
use ketchup_application::program_variants::{self, VariantError};
use ketchup_model::document::RuleProgramSource;
use ketchup_program::model::Param;

fn refused(error: &VariantError) -> &'static str {
    let hint = match error {
        VariantError::EmptyName => "Give the variant a name, e.g. \"800\".",
        VariantError::NoParameters => "Declare the program's sizes with param() first.",
        VariantError::TooMany => "Delete a variant (action=delete_variant) first.",
        VariantError::Unknown { .. } => "Use a name program read lists under variants.",
        VariantError::MissingParameters { .. } => {
            "Delete the variant, or save it again from the current parameters."
        }
    };
    invalid("name", error.to_string(), hint)
}

pub(super) fn owned_program(app: &KetchupApp) -> Result<RuleProgramSource, &'static str> {
    app.document.current_rule_program().cloned().ok_or_else(|| {
        invalid(
            "name",
            "No program owns this document.",
            "Undo a detaching edit or apply a complete program first.",
        )
    })
}

fn params(app: &KetchupApp, program: &RuleProgramSource) -> Result<Vec<Param>, &'static str> {
    // A bridge request answers in the same call.
    app.program_evaluations
        .get_blocking(program)
        .map(|evaluated| evaluated.model.params.clone())
        .map_err(|error| failed_because("program_rejected", error))
}

impl LiveBridge {
    /// The program apply of `use_variant`: `set_params` with the variant's values.
    pub(super) fn expand_use_variant(
        app: &KetchupApp,
        expected: Stamp,
        name: &str,
    ) -> Result<Request, &'static str> {
        Self::guard(app, &Some(expected.clone()))?;
        let program = owned_program(app)?;
        let params = params(app, &program)?;
        let values = program_variants::variant_params(&program, &params, name)
            .map_err(|error| refused(&error))?;
        Self::expand_set_params(app, expected, values)
    }

    /// Saves or deletes a variant as one Undo step that changes no part.
    pub(super) fn program_variant_request(
        &mut self,
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let (expected, name, save) = match request {
            Request::SaveProgramVariant { expected, name } => (expected, name, true),
            Request::DeleteProgramVariant { expected, name } => (expected, name, false),
            request @ Request::ExportProgramVariants { .. } => {
                return Self::export_variants_now(app, request);
            }
            _ => return Err("invalid_request"),
        };
        Self::guard(app, &Some(expected))?;
        Self::available(app, ui_busy)?;
        let program = owned_program(app)?;
        let changed = if save {
            let params = params(app, &program)?;
            program_variants::save_variant(&program, &params, &name)
        } else {
            program_variants::delete_variant(&program, &name)
        }
        .map_err(|error| refused(&error))?;
        let replaced = save && program.variants.contains_key(&name);
        Self::require_request_authority(cancelled)?;
        app.publish_program_variants(changed)
            .map_err(program_failure)?;
        self.pending = None;
        self.query.invalidate();
        let stamp = app.live_bridge_stamp();
        self.observed = Some(stamp.clone());
        let variants = &app
            .document
            .current_rule_program()
            .expect("publishing variants keeps the program")
            .variants;
        let mut result = json!({
            "replaced": replaced,
            "variants": variants,
            "after": stamp,
            "undo_steps": app.undo_step_count(),
        });
        result[if save { "saved" } else { "deleted" }] = name.into();
        Ok(result)
    }
}
