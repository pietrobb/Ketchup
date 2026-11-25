//! The exact half of `program_check`: the program built in a scratch document.
use super::*;

impl Server {
    /// Builds the evaluated program in a scratch document, leaving the
    /// session's document untouched, and settles on its exact solids what the
    /// boxes cannot, exactly as `program_apply` and the window do, by the
    /// deadline of the whole check. A check has no folder to read the STEP
    /// files of bought parts from: it builds without each such part and names
    /// it among what was not evaluated (review 2026-11-24 R1124og-STEP-902).
    pub(super) fn exact_program_check(
        &self,
        source: &ketchup_model::document::RuleProgramSource,
        evaluated: &ketchup_program::Evaluated,
        report: &mut ketchup_program::Report,
        deadline: CheckDeadline,
    ) -> Option<Value> {
        let unread: Vec<Value> = (evaluated.model.parts.iter())
            .filter_map(|part| {
                let file = part.unread_import()?;
                Some(json!({"reason": "import_unread", "part": part.name, "file": file}))
            })
            .collect();
        let mut exact = self.exact_of_read_parts(source, evaluated, report, deadline, &unread)?;
        if !unread.is_empty() {
            exact["state"] = json!("incomplete");
            match exact["not_evaluated"].as_array_mut() {
                Some(reasons) => reasons.extend(unread),
                None => exact["not_evaluated"] = Value::Array(unread),
            }
        }
        Some(exact)
    }

    fn exact_of_read_parts(
        &self,
        source: &ketchup_model::document::RuleProgramSource,
        evaluated: &ketchup_program::Evaluated,
        report: &mut ketchup_program::Report,
        deadline: CheckDeadline,
        unread: &[Value],
    ) -> Option<Value> {
        if ketchup_program::exact_candidates(&evaluated.model).is_empty() {
            return (!unread.is_empty()).then(|| json!({"state": "incomplete"}));
        }
        let mut read_only;
        let built = if unread.is_empty() {
            evaluated.reference_model()
        } else {
            read_only = evaluated.reference_model().clone();
            read_only
                .parts
                .retain(|part| part.unread_import().is_none());
            let kept: std::collections::BTreeSet<String> = read_only
                .parts
                .iter()
                .map(|part| part.name.clone())
                .collect();
            (read_only.joints).retain(|joint| joint.parts.iter().all(|name| kept.contains(name)));
            &read_only
        };
        // The scratch document of a large model takes seconds to build, so it
        // is built once and the build stops between parts at the deadline.
        let past_deadline = || deadline.remaining().is_zero();
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut scratch = DocumentSession::new(self.settings.clone());
        let snapshot = match scratch.build_rule_model_scratch(built, source.clone(), &past_deadline)
        {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => {
                return Some(exact_check_past_deadline(
                    report,
                    &evaluated.model,
                    deadline,
                    &ketchup_program::CancelToken::from(Arc::clone(&cancelled)),
                ));
            }
            Err(error) => {
                let reason = Error::from(error).code;
                return Some(json!({"state": "incomplete",
                "not_evaluated": [{"reason": "program_build_failed", "detail": reason}]}));
            }
        };
        verify_rule_program_exact(
            &snapshot,
            &evaluated.model,
            report,
            scratch.container_data(),
            self.settings.exact_worker_path.clone(),
            deadline,
            cancelled,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::request;
    use super::*;

    /// Review 2026-11-24 R1124og-STEP-902: a check refused to build a program
    /// with a bought part whose file it cannot read, so nothing of it was
    /// checked exactly. It builds the rest and names the part.
    #[test]
    fn a_check_builds_without_a_bought_part_it_cannot_read_and_names_it() {
        // The seat and leg of `program_check_settles_round_contact_on_exact_
        // solids_like_a_build`, with a hinge read from a STEP file.
        let source = "seat = revolve(\"seat\", profile = [(0, 0), (150, 0), (150, 20), (0, 20)], \
                      axis = [(0, 0), (0, 1)], at = (0, 0, 400))\n\
                      rotate(seat, axis = (1, 0, 0), angle = 90)\n\
                      leg = box(\"leg\", (30, 30, 400), at = (120, 120, 0))\n\
                      imported(\"hinge\", \"hinge.step\", at = (130, 130, 200))\n";
        let mut server = Server::new(SessionSettings::default());
        let checked = request(&mut server, "program_check", json!({"source": source}));
        let checked = &checked["result"];
        let exact = &checked["exact"];
        assert_eq!(exact["state"], "incomplete", "{checked}");
        assert_eq!(
            exact["not_evaluated"],
            json!([{"reason": "import_unread", "part": "hinge", "file": "hinge.step"}]),
            "{checked}"
        );
        // The rest was built and measured on its solids.
        assert_eq!(checked["relations"][0]["parts"], json!(["seat", "leg"]));
        assert_eq!(checked["relations"][0]["gap_mm"], 19.7, "{checked}");
        assert!(
            checked["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|issue| issue["kind"] == "import_unread" && issue["parts"] == json!(["hinge"])),
            "{checked}"
        );
    }
}
