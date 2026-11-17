//! Source-bound callouts for circular Pad/Boolean cuts, measured on the current
//! exact body, never on projected circles or cutter travel. The hole about a
//! cut's axis is read from the body's faces ([`profile`]): its bore, counterbores
//! and countersink, and whether it runs through or ends in a flat bottom or a
//! drill point. Coaxial cuts of one hole share one callout. When the body names
//! no face beyond the bore, CYLINDRICAL DEPTH is its axial span and asserts
//! neither end. Threads must suit the bore they are tapped into. Unsupported
//! geometry is reported rather than silently converted into a simpler hole.
use super::*;
use crate::document::{BooleanOperation, FeatureId, FeatureKind};
use ketchup_geometry::sketch::PadOperation;
use profile::{HoleEnd, HoleProfile, HoleUnsupported, hole_profile};

mod profile;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HoleCalloutSource {
    pub feature_id: FeatureId,
    pub instance_token: String,
}

impl DrawingNote {
    /// Present only on notes owned by the exact hole-callout generator.
    #[must_use]
    pub fn hole_callout_sources(&self) -> Option<&[HoleCalloutSource]> {
        self.generated_hole_callout.as_deref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HoleCalloutDiagnosticKind {
    UnsupportedProvenance,
    RemovedOrChangedSource,
    /// No view on the sheet shows the hole as a circle, so a leader could not
    /// point at it.
    NotShownAsCircle,
}

/// A bore's rim: the drawn instance and the exact circular edge.
type Rim = (String, u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoleCalloutDiagnostic {
    pub source: HoleCalloutSource,
    pub kind: HoleCalloutDiagnosticKind,
    pub reason: String,
    pub fix_hint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoleCalloutReport {
    pub sheet: DrawingSheet,
    pub diagnostics: Vec<HoleCalloutDiagnostic>,
}

/// Replaces only notes explicitly owned by this generator. Manual annotations
/// survive unchanged; surviving generated sources retain their IDs and positions.
/// Exact results must be current. No document is mutated. Persist `report.sheet`
/// through the normal sheet edit command. Projection/export revalidates ownership
/// and text, refusing stale callouts until this function is run again.
pub fn generate_hole_callouts(
    snapshot: &Snapshot,
    results: &ExactResultRegistry,
    sheet: &DrawingSheet,
) -> Result<HoleCalloutReport, DrawingError> {
    validate_source(snapshot, sheet.id(), sheet.source())?;
    let instances = source_instances(snapshot, results, sheet.source())?;
    let mut output = sheet.clone();
    output
        .notes
        .retain(|note| note.hole_callout_sources().is_none());
    let drawing = project_orthographic_drawing(snapshot, results, &output)?;
    let (groups, mut diagnostics) = source_groups(snapshot, &instances)?;
    let mut generated: Vec<DrawingNote> = Vec::new();
    let mut next = sheet
        .notes
        .iter()
        .map(|note| note.id().0)
        .max()
        .unwrap_or(0);
    for (label, group) in groups {
        let Some((page_center, page_radius)) = rim_circles(sheet, &drawing.views, &group.rims)
            .first()
            .map(|&(index, circle)| {
                page_circle(
                    sheet,
                    &drawing.layout.view_placements[index],
                    &drawing.views[index],
                    circle,
                )
            })
        else {
            diagnostics.extend(group.sources.into_iter().map(|source| HoleCalloutDiagnostic {
                source,
                kind: HoleCalloutDiagnosticKind::NotShownAsCircle,
                reason: "No view on the sheet shows this hole as a circle, so its callout could not point at it".into(),
                fix_hint: "Add a view that looks along the hole axis, then regenerate the callouts".into(),
            }));
            continue;
        };
        let text = counted_label(&label, group.bores.len());
        let sources = group.sources;
        let previous = sheet.notes.iter().find(|note| {
            note.hole_callout_sources()
                .is_some_and(|old| old.iter().any(|source| sources.contains(source)))
                && !generated.iter().any(|used| used.id() == note.id())
        });
        let mut note = if let Some(old) = previous {
            DrawingNote::new(old.id(), old.position_page_mm(), text)?
        } else {
            next = next.checked_add(1).ok_or(DrawingError::ResourceLimit)?;
            // Up and right of the hole, clear of its rim; the corner list only
            // when that would leave the drawing area.
            let offset = (page_radius + 8.0) * std::f64::consts::FRAC_1_SQRT_2;
            let near = [page_center[0] + offset, page_center[1] + offset];
            let border = drawing.layout.border_bounds_mm;
            let position = if near[0] < border[1][0] - 40.0 && near[1] < border[1][1] - 5.0 {
                near
            } else {
                [
                    20.0,
                    border[1][1]
                        - 10.0
                        - f64::from(u32::try_from(output.notes.len() + generated.len()).map_err(
                            |_error: std::num::TryFromIntError| DrawingError::ResourceLimit,
                        )?) * 6.0,
                ]
            };
            DrawingNote::new(DrawingNoteId(next), position, text)?
        };
        note.generated_hole_callout = Some(sources);
        generated.push(note);
    }
    for old in sheet
        .notes
        .iter()
        .filter_map(DrawingNote::hole_callout_sources)
        .flatten()
    {
        if !generated.iter().any(|note| {
            note.hole_callout_sources()
                .is_some_and(|sources| sources.contains(old))
        }) && !diagnostics.iter().any(|diagnostic| {
            diagnostic.kind == HoleCalloutDiagnosticKind::NotShownAsCircle
                && &diagnostic.source == old
        }) {
            diagnostics.push(HoleCalloutDiagnostic {
                source: old.clone(),
                kind: HoleCalloutDiagnosticKind::RemovedOrChangedSource,
                reason: "Previous generated callout removed because its exact source is no longer supported".into(),
                fix_hint: "Review the changed cut and regenerate after restoring supported exact geometry".into(),
            });
        }
    }
    output.notes.extend(generated);
    if output.notes.len() > MAX_DRAWING_NOTES {
        return Err(DrawingError::ResourceLimit);
    }
    project_orthographic_drawing(snapshot, results, &output)?;
    Ok(HoleCalloutReport {
        sheet: output,
        diagnostics,
    })
}

#[derive(Default)]
struct CalloutGroup {
    sources: Vec<HoleCalloutSource>,
    bores: std::collections::BTreeSet<(String, u32)>,
    rims: std::collections::BTreeSet<Rim>,
}

/// One hole of an instance and the cuts on its axis.
struct Hole {
    profile: HoleProfile,
    /// Each cut with the depth of the hole's entry from its start.
    cuts: Vec<(HoleCalloutSource, f64)>,
}

fn source_groups(
    snapshot: &Snapshot,
    instances: &[DrawingInstance],
) -> Result<(BTreeMap<String, CalloutGroup>, Vec<HoleCalloutDiagnostic>), DrawingError> {
    let mut groups: BTreeMap<String, CalloutGroup> = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for instance in instances {
        // By the first face of each hole on this instance's body, in the order
        // of each hole's first cut so sources keep feature order.
        let mut holes: Vec<(u32, Hole)> = Vec::new();
        for feature in snapshot.features().filter(|feature| {
            feature.definition_id() == instance.package.definition_id()
                && (is_cut(feature.kind())
                    || snapshot.hole_threads().any(|(id, _)| id == feature.id()))
        }) {
            let source = HoleCalloutSource {
                feature_id: feature.id(),
                instance_token: instance.token.clone(),
            };
            match feature_hole(snapshot, instance, feature.id()) {
                Ok(profile) => {
                    let Some(&key) = profile.faces.first() else {
                        continue;
                    };
                    let entry = profile.entry;
                    if let Some((_, hole)) = holes.iter_mut().find(|(face, _)| *face == key) {
                        hole.cuts.push((source, entry));
                    } else {
                        holes.push((
                            key,
                            Hole {
                                profile,
                                cuts: vec![(source, entry)],
                            },
                        ));
                    }
                }
                Err(reason) => {
                    let mut diagnostic = unsupported(source);
                    if let Some(reason) = reason {
                        diagnostic.reason = reason.to_string();
                    }
                    diagnostics.push(diagnostic);
                }
            }
        }
        for (key, hole) in holes {
            let label = match hole_label(snapshot, &hole) {
                Ok(label) => label,
                Err(reason) => {
                    diagnostics.extend(hole.cuts.into_iter().map(|(source, _)| {
                        let mut diagnostic = unsupported(source);
                        diagnostic.reason = reason.to_string();
                        diagnostic
                    }));
                    continue;
                }
            };
            let group = groups.entry(label).or_default();
            group.bores.insert((instance.token.clone(), key));
            group.rims.extend(
                hole.profile
                    .rims
                    .iter()
                    .map(|edge| (instance.token.clone(), *edge)),
            );
            group
                .sources
                .extend(hole.cuts.into_iter().map(|(source, _)| source));
        }
    }
    Ok((groups, diagnostics))
}

fn counted_label(label: &str, count: usize) -> String {
    if count == 1 {
        label.to_owned()
    } else {
        format!("{count}× {label}")
    }
}

fn is_cut(kind: &FeatureKind) -> bool {
    matches!(kind, FeatureKind::Pad(pad) if matches!(pad.operation, PadOperation::Cut { .. }))
        || matches!(
            kind,
            FeatureKind::Boolean {
                operation: BooleanOperation::Cut,
                ..
            }
        )
}

fn unsupported(source: HoleCalloutSource) -> HoleCalloutDiagnostic {
    HoleCalloutDiagnostic {
        source,
        kind: HoleCalloutDiagnosticKind::UnsupportedProvenance,
        reason: "Callout requires a circular cut on the final body's chain of cuts and finishes, whose bore the finished body keeps whole".into(),
        fix_hint: "Use a circular Pad/Boolean cut for the hole; otherwise provide a reviewed manual annotation".into(),
    }
}

/// Following only cuts and edge finishes cannot move the original bore; what
/// they did to it is read from the finished exact body.
fn on_cut_chain(snapshot: &Snapshot, mut producer: FeatureId, source: FeatureId) -> bool {
    loop {
        if producer == source {
            return true;
        }
        producer = match snapshot.feature(producer).map(|feature| feature.kind()) {
            Some(FeatureKind::Pad(pad)) if matches!(pad.operation, PadOperation::Cut { .. }) => {
                match pad.operation.target() {
                    Some(target) => target,
                    None => return false,
                }
            }
            Some(FeatureKind::Boolean {
                operation: BooleanOperation::Cut,
                target,
                ..
            })
            | Some(FeatureKind::EdgeFinish { target, .. }) => *target,
            _ => return false,
        };
    }
}

/// The hole about the circular cut `id`, read from the instance's finished
/// body; `Err(None)` when `id` is no circular cut on that body's chain.
fn feature_hole(
    snapshot: &Snapshot,
    instance: &DrawingInstance,
    id: FeatureId,
) -> Result<HoleProfile, Option<HoleUnsupported>> {
    let Some(feature) = snapshot.feature(id) else {
        return Err(None);
    };
    if !is_cut(feature.kind())
        || !on_cut_chain(snapshot, instance.package.producer_feature_id(), id)
    {
        return Err(None);
    }
    let Some((start, direction, diameter)) = snapshot.circular_cut_axis(id) else {
        return Err(None);
    };
    let ExactBodyPackage::Graph(package) = instance.package.as_ref() else {
        return Err(None);
    };
    hole_profile(
        start,
        direction,
        diameter,
        &package.edge_evidence,
        &package.face_evidence,
    )
    .map_err(Some)
}

/// The callout of `hole`: its thread, its bore and how it ends, then its
/// counterbores and countersink. Depths are from the hole's entry.
fn hole_label(snapshot: &Snapshot, hole: &Hole) -> Result<String, HoleUnsupported> {
    let profile = &hole.profile;
    let from_entry = |depth: f64| depth - profile.entry;
    let Some(bore) = profile.bores.last() else {
        return Err(HoleUnsupported::NoBore);
    };
    let threaded = hole
        .cuts
        .iter()
        .filter(|(source, _)| {
            snapshot
                .hole_threads()
                .any(|(id, _)| id == source.feature_id)
        })
        .collect::<Vec<_>>();
    let thread = match threaded.as_slice() {
        [] => None,
        [(source, entry)] => Some(thread_label(snapshot, profile, source.feature_id, *entry)?),
        _ => return Err(HoleUnsupported::TwoThreads),
    };
    let diameter = number(bore.diameter);
    let depth = number(from_entry(bore.to));
    let only_bore = profile.bores.len() == 1 && profile.countersink.is_none();
    let mut parts = thread.into_iter().collect::<Vec<_>>();
    parts.push(match profile.end {
        HoleEnd::Through => format!("BORE Ø{diameter} mm THRU"),
        HoleEnd::FlatBottom => format!("BORE Ø{diameter} mm DEPTH {depth} mm, FLAT BOTTOM"),
        HoleEnd::Point(angle) => format!(
            "BORE Ø{diameter} mm DEPTH {depth} mm, {}° POINT",
            number(angle)
        ),
        HoleEnd::Unknown if !parts.is_empty() => format!("BORE Ø{diameter} mm"),
        HoleEnd::Unknown if only_bore => {
            format!("BORE Ø{diameter} mm; CYLINDRICAL DEPTH {depth} mm")
        }
        HoleEnd::Unknown => format!("BORE Ø{diameter} mm CYLINDRICAL DEPTH {depth} mm"),
    });
    for counterbore in &profile.bores[..profile.bores.len() - 1] {
        let mut part = format!(
            "COUNTERBORE Ø{} mm DEPTH {} mm",
            number(counterbore.diameter),
            number(from_entry(counterbore.to))
        );
        if let Some(angle) = counterbore.step_angle {
            part.push_str(&format!(", {}° STEP", number(angle)));
        }
        parts.push(part);
    }
    if let Some((diameter, angle)) = profile.countersink {
        parts.push(format!(
            "COUNTERSINK Ø{} mm × {}°",
            number(diameter),
            number(angle)
        ));
    }
    Ok(parts.join("; "))
}

/// The thread tapped by the cut `id` into the bore of its diameter, which the
/// thread must start at; `entry` is the hole's entry from the cut's start.
fn thread_label(
    snapshot: &Snapshot,
    profile: &HoleProfile,
    id: FeatureId,
    entry: f64,
) -> Result<String, HoleUnsupported> {
    let tolerance = ketchup_tolerance::APPROXIMATION;
    let Some(thread) = snapshot.tapped_hole(id) else {
        return Err(HoleUnsupported::ThreadNotCircle);
    };
    // Depths from the cut's start, where the thread starts.
    let from_cut = |depth: f64| depth - profile.entry + entry;
    let last = profile.bores.len() - 1;
    let Some((index, bore)) = profile
        .bores
        .iter()
        .enumerate()
        .find(|(_, bore)| (bore.diameter - thread.surface_diameter_mm).abs() < tolerance)
    else {
        return Err(HoleUnsupported::NoThreadBore);
    };
    if from_cut(bore.from).abs() > tolerance {
        return Err(HoleUnsupported::ThreadStartsElsewhere);
    }
    ketchup_geometry::tapping::TappedHole {
        size: &thread.size,
        pitch_mm: thread.pitch_mm,
        thread_depth_mm: thread.length_mm,
        hole_diameter_mm: bore.diameter,
        hole_depth_mm: from_cut(bore.to),
        through: index == last && profile.end == HoleEnd::Through,
    }
    .check(ketchup_tolerance::APPROXIMATION)
    .map_err(HoleUnsupported::Thread)?;
    Ok(format!(
        "{} x {} THREAD DEPTH {} mm",
        thread.size,
        number(thread.pitch_mm),
        number(thread.length_mm)
    ))
}

// Suppress floating-point noise in exact measurements, not manufacturing units.
fn number(value: f64) -> String {
    format!("{value:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

/// Which generated note points at which rims. Every hole group the sheet
/// shows as a circle must have exactly one current generated note, and every
/// generated note one such group; groups no view shows have no note.
pub(super) fn validate_owned(
    snapshot: &Snapshot,
    sheet: &DrawingSheet,
    instances: &[DrawingInstance],
    views: &[OrthographicView],
) -> Result<BTreeMap<DrawingNoteId, std::collections::BTreeSet<Rim>>, DrawingError> {
    let generated = sheet
        .notes
        .iter()
        .filter(|note| note.hole_callout_sources().is_some())
        .collect::<Vec<_>>();
    let mut anchors = BTreeMap::new();
    if generated.is_empty() {
        return Ok(anchors);
    }
    let (groups, _) = source_groups(snapshot, instances)?;
    let shown = groups
        .into_iter()
        .filter(|(_, group)| !rim_circles(sheet, views, &group.rims).is_empty())
        .collect::<Vec<_>>();
    if generated.len() != shown.len() {
        return Err(DrawingError::AnnotationSourceLost);
    }
    for (label, group) in shown {
        let text = counted_label(&label, group.bores.len());
        let note = generated
            .iter()
            .find(|note| {
                note.text_template() == text
                    && note.hole_callout_sources().is_some_and(|sources| {
                        sources.len() == group.sources.len()
                            && group.sources.iter().all(|source| sources.contains(source))
                    })
            })
            .ok_or(DrawingError::AnnotationSourceLost)?;
        anchors.insert(note.id(), group.rims);
    }
    Ok(anchors)
}

/// The full projected circles of `rims`, as (view index, circle), in sheet
/// view order. Section views project no circles.
fn rim_circles<'a>(
    sheet: &DrawingSheet,
    views: &'a [OrthographicView],
    rims: &std::collections::BTreeSet<Rim>,
) -> Vec<(usize, &'a ProjectedCircle)> {
    let mut output = Vec::new();
    for (index, view) in views.iter().enumerate() {
        let ids = rims
            .iter()
            .map(|(token, edge)| {
                format!(
                    "sheet-{}/view-{}/{token}:circle:{edge}",
                    sheet.id().0,
                    view.kind.stable_name()
                )
            })
            .collect::<std::collections::BTreeSet<_>>();
        output.extend(
            view.circles
                .iter()
                .filter(|circle| circle.span_rad >= TAU && ids.contains(&circle.stable_circle_id))
                .map(|circle| (index, circle)),
        );
    }
    output
}

/// A projected circle's page centre and radius.
fn page_circle(
    sheet: &DrawingSheet,
    placement: &DrawingViewPlacement,
    view: &OrthographicView,
    circle: &ProjectedCircle,
) -> ([f64; 2], f64) {
    let scale = sheet.page().scale().factor() * view.kind.layout_scale_multiplier();
    (
        [
            placement.origin_mm[0] + circle.center_mm[0] * scale,
            placement.origin_mm[1] + circle.center_mm[1] * scale,
        ],
        circle.radius_mm * scale,
    )
}

/// The leader from a generated callout at `from` to the nearest point on the
/// nearest projected rim of its holes, with that rim's circle id.
pub(super) fn callout_leader(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    rims: &std::collections::BTreeSet<Rim>,
    from: [f64; 2],
) -> Result<(String, [[f64; 2]; 2]), DrawingError> {
    rim_circles(sheet, views, rims)
        .into_iter()
        .map(|(index, circle)| {
            let (center, radius) = page_circle(sheet, &placements[index], &views[index], circle);
            let offset = [from[0] - center[0], from[1] - center[1]];
            let length = offset[0].hypot(offset[1]);
            let direction = if length > INTERSECTION_EPSILON {
                [offset[0] / length, offset[1] / length]
            } else {
                [std::f64::consts::FRAC_1_SQRT_2; 2]
            };
            let end = [
                center[0] + direction[0] * radius,
                center[1] + direction[1] * radius,
            ];
            (
                (length - radius).abs(),
                circle.stable_circle_id.clone(),
                [from, end],
            )
        })
        .min_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
        })
        .map(|(_, id, leader)| (id, leader))
        .ok_or(DrawingError::AnnotationSourceLost)
}
