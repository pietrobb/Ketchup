//! The Fit underlay tool: two clicks on the lines of one underlay, then two clicks on the
//! model, place that underlay so the first pair lands on the second ([`KetchupApp::fit_underlay`]),
//! as one Undo step. Scaling is a menu option, off by default.

use crate::*;
use ketchup_model::underlay::UnderlayId;

/// The points picked so far while the Fit underlay tool is active.
#[derive(Default)]
pub(crate) struct UnderlayFitGesture {
    /// The underlay the first click picked, and its picked points in document XY.
    pub(crate) from: Option<(UnderlayId, Vec<[f64; 2]>)>,
    /// The picked model points in document XY.
    pub(crate) to: Vec<[f64; 2]>,
    /// The document revision the points were picked on: once it changes (an MCP edit
    /// among them) they may no longer lie where the lines and the model are.
    revision: u64,
    /// The fit also scales the underlay; kept between fits.
    pub(crate) scaled: bool,
}

impl UnderlayFitGesture {
    pub(crate) fn clear_points(&mut self) {
        self.from = None;
        self.to.clear();
    }
}

impl KetchupApp {
    /// The best snap on the shown lines of an underlay under `pointer`, of `only` if given,
    /// ranked like the model's snaps.
    fn underlay_point_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        only: Option<UnderlayId>,
    ) -> Option<(UnderlayId, Vec3)> {
        self.underlay_snaps_at_screen(pointer, rect, 8.0)
            .into_iter()
            .filter_map(|(reference, kind, point)| match reference.element {
                ElementId::Underlay { underlay, .. } if only.is_none_or(|id| id == underlay) => {
                    let rank = SnapResult {
                        kind,
                        reference,
                        position_mm: point,
                        distance_mm: 0.0,
                    }
                    .score()
                    .kind_rank;
                    Some((
                        rank,
                        self.project(point, rect).distance(pointer),
                        underlay,
                        point,
                    ))
                }
                _ => None,
            })
            .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.total_cmp(&b.1)))
            .map(|(_, _, underlay, point)| (underlay, point))
    }

    /// One click of the Fit underlay tool: the first two pick points on one underlay's
    /// lines, the next two model points; the fourth places the underlay.
    pub(crate) fn pick_underlay_fit_point(&mut self, pointer: Pos2, rect: Rect) {
        let revision = self.document_revision();
        if self.gesture.underlay_fit.from.is_some()
            && self.gesture.underlay_fit.revision != revision
        {
            self.gesture.underlay_fit.clear_points();
            self.status_key = "status-fit-underlay-first-line";
            self.digest = self.catalog.text("digest-fit-underlay-picks-dropped");
            return;
        }
        self.gesture.underlay_fit.revision = revision;
        let picked_from = self
            .gesture
            .underlay_fit
            .from
            .as_ref()
            .map_or(0, |f| f.1.len());
        if picked_from < 2 {
            let only = self.gesture.underlay_fit.from.as_ref().map(|f| f.0);
            let Some((id, point)) = self.underlay_point_at_screen(pointer, rect, only) else {
                self.digest = self.catalog.text("digest-fit-underlay-line-required");
                return;
            };
            self.gesture
                .underlay_fit
                .from
                .get_or_insert_with(|| (id, Vec::new()))
                .1
                .push([point.x, point.y]);
            self.status_key = if picked_from == 0 {
                "status-fit-underlay-second-line"
            } else {
                "status-fit-underlay-first-model"
            };
            return;
        }
        let Some(point) = self.measurement_point_at_screen(pointer, rect, 0.0) else {
            self.digest = self.catalog.text("digest-fit-underlay-model-required");
            return;
        };
        self.gesture.underlay_fit.to.push([point.x, point.y]);
        if self.gesture.underlay_fit.to.len() < 2 {
            self.status_key = "status-fit-underlay-second-model";
            return;
        }
        let fit = &mut self.gesture.underlay_fit;
        let (id, from) = fit
            .from
            .take()
            .expect("two underlay points are picked first");
        let to = std::mem::take(&mut fit.to);
        let scaled = fit.scaled;
        self.status_key = "status-fit-underlay-first-line";
        self.fit_underlay_in_window(id, [from[0], from[1]], [to[0], to[1]], scaled);
    }

    /// The fit the tool's fourth click makes: once the regions of the areas that follow the
    /// drawing are found off the UI thread (review 2026-10-29 UO-40).
    pub(crate) fn fit_underlay_in_window(
        &mut self,
        id: UnderlayId,
        from: [[f64; 2]; 2],
        to: [[f64; 2]; 2],
        scaled: bool,
    ) {
        if let Ok((batch, _)) = ketchup_model::underlay::placement::plan_underlay_fit(
            &self.document.current(),
            id,
            from,
            to,
            scaled,
        ) && !self.followed_regions_or_await(&batch, move |app| {
            app.fit_underlay_in_window(id, from, to, scaled);
        }) {
            return;
        }
        self.digest = match self.fit_underlay(id, from, to, scaled) {
            Ok(fit) => self.catalog.format(
                "digest-fit-underlay-done",
                &BTreeMap::from([
                    ("turn", format!("{:.3}", fit.turn_deg)),
                    ("scale", format!("{:.6}", fit.scale)),
                    ("mismatch", format_height(fit.length_mismatch_mm)),
                ]),
            ),
            Err(error) => self.catalog.format(
                "digest-fit-underlay-failed",
                &BTreeMap::from([("reason", error.to_string())]),
            ),
        };
    }
}
