//! Small program reads, revision-bound source patches and complete report pages.
use super::*;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceEdit {
    pub old: String,
    pub new: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportSection {
    CutList,
    Hardware,
    Machining,
    Relations,
    Issues,
    MaterialTakeoff,
    SelectionTakeoff,
    Joints,
    Loads,
    Members,
}

pub(super) fn invalid(target: &str, reason: impl Into<String>, hint: &str) -> &'static str {
    record_rejection(
        &rejected("invalid_params")
            .target(target)
            .reason(reason)
            .fix_hint(hint),
        json!({"published": false}),
    );
    "invalid_params"
}

/// Most source edits one patch carries; the tool schema states the same bound.
const PATCH_EDITS: usize = 100;

/// Resolve every edit against the original text, never against another edit's output.
fn patched(source: &str, edits: &[SourceEdit]) -> Result<String, &'static str> {
    if edits.is_empty() || edits.len() > PATCH_EDITS {
        return Err(invalid(
            "edits",
            "Expected 1 to 100 source edits.",
            "Provide unique old/new text pairs.",
        ));
    }
    let mut ranges = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        // char boundaries also find overlapping matches (e.g. 'aa' in 'aaa').
        let matches = source
            .char_indices()
            .filter_map(|(start, _)| source[start..].starts_with(&edit.old).then_some(start))
            .take(2)
            .collect::<Vec<_>>();
        let field = format!("edits[{index}].old");
        if edit.old.is_empty() {
            return Err(invalid(
                &field,
                "Old text is empty.",
                "Give the exact text to replace, with enough surrounding text to identify one occurrence.",
            ));
        }
        match matches.len() {
            0 => {
                return Err(invalid(
                    &field,
                    "Old text does not occur in the current source.",
                    "Read the source again (it may have changed) and copy the old text exactly, including whitespace.",
                ));
            }
            1 => {}
            _ => {
                return Err(invalid(
                    &field,
                    "Old text occurs more than once in the current source.",
                    "Include enough surrounding text to identify one occurrence.",
                ));
            }
        }
        ranges.push((matches[0], matches[0] + edit.old.len(), &edit.new));
    }
    ranges.sort_by_key(|range| range.0);
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(invalid(
            "edits",
            "Source edits overlap.",
            "Combine overlapping edits into one old/new pair.",
        ));
    }
    let mut output = source.to_owned();
    for (start, end, replacement) in ranges.into_iter().rev() {
        output.replace_range(start..end, replacement);
    }
    Ok(output)
}

pub(super) fn source_diff(before: &str, after: &str) -> Value {
    let before: Vec<_> = before.lines().collect();
    let after: Vec<_> = after.lines().collect();
    let first = before
        .iter()
        .zip(&after)
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = before[first..]
        .iter()
        .rev()
        .zip(after[first..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old = before[first..before.len() - suffix].join("\n");
    let new = after[first..after.len() - suffix].join("\n");
    json!({"first_line": first + 1, "removed_lines": before.len() - first - suffix,
        "added_lines": after.len() - first - suffix,
        "old": old.chars().take(600).collect::<String>(), "new": new.chars().take(600).collect::<String>(),
        "truncated": old.chars().count() > 600 || new.chars().count() > 600})
}

impl LiveBridge {
    pub(super) fn read_program_request(
        &mut self,
        app: &KetchupApp,
        request: Request,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        match request {
            Request::Program { expected } => {
                Self::guard(app, &expected)?;
                Ok(app.program_source_view().unwrap_or_else(|| json!({"source":null,"hint":"No program owns this document; undo a detaching edit or apply a complete program."})))
            }
            Request::ProgramContext {
                expected,
                selection_context,
            } => {
                Self::guard(app, &expected)?;
                Self::program_context(app, selection_context)
            }
            Request::ProgramPiece {
                expected,
                lines,
                search,
                part,
                outline,
            } => {
                let piece =
                    super::program_navigation::Piece::from_request(lines, search, part, outline)?;
                Self::program_piece(app, &expected, &piece)
            }
            Request::ProgramReport {
                expected,
                section,
                offset,
                limit,
            } => self.program_report_page(app, expected, section, offset, limit),
            Request::MeasureFaces {
                expected,
                faces,
                mode,
                direction,
            } => self.measure_now(
                app,
                expected,
                faces,
                mode,
                direction,
                Arc::new(AtomicBool::new(cancelled.load(Ordering::Acquire))),
            ),
            Request::ValidateProgram {
                expected,
                validators,
                motion,
            } => self.validate_program(
                app,
                expected,
                Arc::new(AtomicBool::new(cancelled.load(Ordering::Acquire))),
                validators,
                motion,
            ),
            _ => Err(invalid(
                "action",
                "Expected a read-only program or measurement request.",
                "Use program read/report/validate or inspect measure.",
            )),
        }
    }
    pub(super) fn expand_program_patch(
        app: &KetchupApp,
        request: Request,
    ) -> Result<Request, &'static str> {
        let (expected, edits) = match request {
            Request::PatchProgram { expected, edits } => (expected, edits),
            Request::SetProgramParams { expected, params } => {
                return Self::expand_set_params(app, expected, params);
            }
            request => return Ok(request),
        };
        Self::guard(app, &Some(expected.clone()))?;
        let program = app.document.current_rule_program().ok_or_else(|| {
            invalid(
                "source",
                "No program owns this document.",
                "Undo a detaching edit or apply a complete program first.",
            )
        })?;
        Ok(Request::ApplyProgram {
            expected: Some(expected),
            source: patched(&program.source, &edits)?,
            overrides: Some(program.overrides.clone()),
            file_name: Some(program.file_name.clone()),
            replace_document: false,
        })
    }

    pub(super) fn program_context(
        app: &KetchupApp,
        selection_context: bool,
    ) -> Result<Value, &'static str> {
        let Some(program) = app.document.current_rule_program() else {
            return Ok(
                json!({"source": null, "hint": "No program owns this document; undo a detaching edit or apply a complete program."}),
            );
        };
        if !selection_context {
            return Ok(
                json!({"file_name": program.file_name, "source": program.source, "overrides": program.overrides}),
            );
        }
        let snapshot = app.document.current();
        let selected: BTreeSet<_> = app
            .selected_instance_paths()
            .iter()
            .filter_map(|path| ketchup_application::rule_program_part_name(&snapshot, path))
            .collect();
        // A bridge request answers in the same call.
        let evaluated = app
            .program_evaluations
            .get_blocking(program)
            .map_err(|error| failed_because("program_rejected", error))?;
        let sources = &evaluated.part_sources;
        let lines: Vec<_> = program.source.split_inclusive('\n').collect();
        let mut included = BTreeSet::new();
        let mut parts = Vec::new();
        for (name, ranges) in sources.iter().filter(|(name, _)| selected.contains(*name)) {
            let ranges: Vec<_> = ranges
                .iter()
                .map(|range| [range.first, range.last])
                .collect();
            for [first, last] in &ranges {
                included.extend(first.saturating_sub(3)..last.saturating_add(2).min(lines.len()));
            }
            parts.push(json!({"name": name, "lines": ranges}));
        }
        let snippets: Vec<_> = included
            .into_iter()
            .filter_map(|index| {
                lines
                    .get(index)
                    .map(|text| json!({"line": index + 1, "text": text}))
            })
            .collect();
        Ok(
            json!({"file_name": program.file_name, "overrides": program.overrides,
            "parts": parts, "lines": snippets, "selected_context": Self::selected_context(app),
            "hint": "Line text retains line endings: concatenate directly for patch old text. Includes two neighbours, not a dependency closure. Reuse known context; read mode=source for missing dependencies."}),
        )
    }

    pub(super) fn program_report_page(
        &mut self,
        app: &KetchupApp,
        expected: Stamp,
        section: ReportSection,
        offset: usize,
        limit: usize,
    ) -> Result<Value, &'static str> {
        Self::guard(app, &Some(expected.clone()))?;
        if let ReportSection::MaterialTakeoff | ReportSection::SelectionTakeoff = section {
            check_page_limit(limit)?;
            let selected = matches!(section, ReportSection::SelectionTakeoff);
            if selected && app.selected_instance_paths().is_empty() {
                return Err(invalid(
                    "selection",
                    "Nothing is selected.",
                    "Select parts with view action=selection, or use material_takeoff for all visible parts.",
                ));
            }
            // A bridge request answers in the same call.
            app.wait_for_program_evaluation();
            let takeoff = if selected {
                app.selected_material_takeoff().map(Arc::new)
            } else {
                app.material_takeoff()
            }
            .map_err(|reason| {
                invalid(
                    "source",
                    app.takeoff_error_text(&reason),
                    "Apply a program; the takeoff counts the parts of a program-owned document.",
                )
            })?;
            let basis = if selected {
                "selected_visible_program_parts"
            } else {
                "visible_program_parts"
            };
            return page(
                section,
                &takeoff_rows(&takeoff),
                offset,
                limit,
                json!({"basis": basis, "counted_parts": takeoff.counted_parts,
                    "excluded_parts": takeoff.excluded_parts, "exact_volume_parts": takeoff.exact_volume_parts,
                    "outside_program_parts": takeoff.outside_program_parts,
                    "hint": "Only visible program parts count: parts on hidden layers (tags) are excluded, and outside_program_parts visible parts not made by the program (drawn or imported) are not in the takeoff. Sizes are blanks; volume_basis says whether volume and area are the exact solid's or the blank's."}),
            );
        }
        if self.program_check_job.is_some() {
            return Err(failure(
                "busy",
                "Wait for the program check before paging its report.",
                json!({}),
            ));
        }
        check_page_limit(limit)?;
        if self
            .program_report
            .as_ref()
            .is_none_or(|(stamp, _, _)| stamp != &expected)
        {
            let source = app.document.current_rule_program().ok_or_else(|| {
                invalid(
                    "source",
                    "No program owns this document.",
                    "Use inspect query for a detached document.",
                )
            })?;
            let (_, report) =
                ketchup_program::run(&source.file_name, &source.source, &source.overrides)
                    .map_err(|error| failed_because("program_rejected", error))?;
            self.program_report = Some((expected.clone(), report, "program_evaluation"));
        }
        let Some((_, report, basis)) = self.program_report.as_ref() else {
            return Err(invalid(
                "report",
                "No report is available.",
                "Read the current program before requesting report pages.",
            ));
        };
        report_page(report, section, offset, limit, basis)
    }
}

/// Flatten unbounded nested lists so even one repeated part or heavily machined part can be paged.
fn report_rows(report: &ketchup_program::Report, section: ReportSection) -> Vec<Value> {
    match section {
        ReportSection::CutList => report
            .bom
            .cut_list
            .iter()
            .enumerate()
            .flat_map(|(group, row)| {
                row.parts.iter().map(move |part| {
                    json!({"group": group, "material": row.material,
                "dimensions_mm": row.dimensions_mm, "group_count": row.count, "part": part})
                })
            })
            .collect(),
        ReportSection::Hardware => report.bom.hardware.iter().map(|row| json!(row)).collect(),
        ReportSection::Machining => report
            .bom
            .machining
            .iter()
            .flat_map(|part| {
                part.operations.iter().map(move |operation| {
                    json!({"part": part.part,
                "size_mm": part.size_mm, "operation": operation})
                })
            })
            .collect(),
        ReportSection::Relations => report.relations.iter().map(|row| json!(row)).collect(),
        ReportSection::Issues => report.issues.iter().map(|row| json!(row)).collect(),
        ReportSection::MaterialTakeoff | ReportSection::SelectionTakeoff => Vec::new(),
        ReportSection::Joints => report.joints.iter().map(|row| json!(row)).collect(),
        ReportSection::Loads => report.loads.members.iter().map(|row| json!(row)).collect(),
        ReportSection::Members => report.design.members.iter().map(|row| json!(row)).collect(),
    }
}

fn check_page_limit(limit: usize) -> Result<(), &'static str> {
    if (1..=100).contains(&limit) {
        Ok(())
    } else {
        Err(invalid(
            "limit",
            "Page limit must be 1 to 100.",
            "Use limit=50 and follow next_offset until null.",
        ))
    }
}

/// Category/material/cross-section rows first, then one total per material.
fn takeoff_rows(takeoff: &ketchup_program::takeoff::Takeoff) -> Vec<Value> {
    takeoff
        .rows
        .iter()
        .map(|row| {
            json!({"kind": "row", "category": row.category, "material": row.material,
            "section_mm": row.section_mm, "count": row.count, "length_m": row.length_m,
            "area_m2": row.area_m2, "volume_m3": row.volume_m3, "volume_basis": row.volume_basis,
            "parts": row.parts.len()})
        })
        .chain(takeoff.materials.iter().map(|total| {
            let mut value = json!(total);
            value["kind"] = json!("material_total");
            value
        }))
        .collect()
}

fn report_page(
    report: &ketchup_program::Report,
    section: ReportSection,
    offset: usize,
    limit: usize,
    basis: &str,
) -> Result<Value, &'static str> {
    page(
        section,
        &report_rows(report, section),
        offset,
        limit,
        json!({"basis": basis, "total_parts": report.bom.total_parts,
            "hint": "program_evaluation is not a new native geometry check; unverified is not wrong or passed."}),
    )
}

fn page(
    section: ReportSection,
    rows: &[Value],
    offset: usize,
    limit: usize,
    mut extra: Value,
) -> Result<Value, &'static str> {
    if offset > rows.len() {
        return Err(invalid(
            "offset",
            "Offset exceeds the section's row count.",
            "Use the returned next_offset for the same revision and section.",
        ));
    }
    let mut page = Vec::new();
    let mut bytes = 0;
    for row in rows.iter().skip(offset).take(limit) {
        let size = row.to_string().len();
        if bytes + size > 24 * 1024 {
            if page.is_empty() {
                return Err(invalid(
                    "section",
                    "One report row exceeds the bridge frame budget.",
                    "Shorten unusually long part names or issue messages in the source.",
                ));
            }
            break;
        }
        page.push(row.clone());
        bytes += size;
    }
    let next = offset + page.len();
    extra["section"] = json!(section);
    extra["rows"] = json!(page);
    extra["total"] = json!(rows.len());
    extra["offset"] = json!(offset);
    extra["next_offset"] = json!((next < rows.len()).then_some(next));
    Ok(extra)
}
