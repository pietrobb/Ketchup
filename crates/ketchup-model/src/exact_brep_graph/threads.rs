//! Modelled threads in the graph: the helical groove of a thread the
//! document models, cut into the body its feature makes as the node right
//! after that feature's own, under the same feature.
use super::*;
use crate::document::metric_nominal_diameter;
use ketchup_geometry::linalg::Vec3;
use ketchup_geometry::thread_profile::{MetricThread, ThreadPhase};

/// Whether `node` cuts the thread of the feature whose body the node right
/// before it made: the one node that may repeat a feature.
pub(super) fn threads_previous_node(node: &ExactBRepNode, prior_nodes: &[ExactBRepNode]) -> bool {
    matches!(
        (&node.operation, prior_nodes.last()),
        (ExactBRepOperation::ThreadGroove(thread), Some(previous))
            if thread.target == previous.id
                && previous.source_feature_id == node.source_feature_id
                && !matches!(previous.operation, ExactBRepOperation::ThreadGroove(_))
    )
}

impl ExactBRepGraph {
    /// The graph with its modelled threads cosmetic: every thread groove left
    /// out, so the body is the one the document makes with cosmetic threads
    /// (drawings show a modelled thread by ISO 6410, as a cosmetic one).
    /// `None` when the graph models no thread.
    ///
    /// # Errors
    /// The graph cannot be serialised for its digest.
    pub fn without_thread_grooves(&self) -> Result<Option<Self>, ExactBRepGraphError> {
        if !self
            .nodes
            .iter()
            .any(|node| matches!(node.operation, ExactBRepOperation::ThreadGroove(_)))
        {
            return Ok(None);
        }
        // Each old node's id in the new graph; a groove stands for its target.
        let mut renumbered = Vec::<ExactBRepNodeId>::with_capacity(self.nodes.len());
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if let ExactBRepOperation::ThreadGroove(groove) = &node.operation {
                renumbered.push(renumbered[groove.target.index()]);
                continue;
            }
            let mut node = node.clone();
            for dependency in node.operation.dependencies_mut() {
                *dependency = renumbered[dependency.index()];
            }
            node.id = ExactBRepNodeId(
                nodes
                    .len()
                    .try_into()
                    .map_err(|_: std::num::TryFromIntError| ExactBRepGraphError::ResourceLimit)?,
            );
            renumbered.push(node.id);
            nodes.push(node);
        }
        let mut graph = Self {
            schema: graph_schema(&nodes, &self.profiles).to_owned(),
            nodes,
            ..self.clone()
        };
        graph.graph_digest = graph.compute_graph_digest()?;
        graph.canonical_input_digest = graph.compute_canonical_input_digest();
        Ok(Some(graph))
    }
}

/// A modelled thread in the definition's frame.
struct ModeledThread {
    metric: MetricThread,
    phase: ThreadPhase,
    start: Vec3,
    end: Vec3,
    surface_diameter_mm: f64,
    internal: bool,
}

impl GraphCompiler<'_> {
    /// The node of the body `feature_id` makes: `node`, or with the thread
    /// the document models on it cut into it. A modelled thread whose
    /// geometry cannot be built is an error, never a cosmetic thread.
    pub(super) fn threaded(
        &mut self,
        feature_id: FeatureId,
        node: ExactBRepNodeId,
    ) -> Result<ExactBRepNodeId, ExactBRepGraphError> {
        let Some(thread) = self.modeled_thread(feature_id)? else {
            return Ok(node);
        };
        let bounds = self.node_bounds[node.index()];
        let groove = thread
            .groove(
                node,
                bounds.ok_or(ExactBRepGraphError::UnresolvedExtent)?,
                self.snapshot.tolerance().linear_mm(),
            )
            .ok_or(ExactBRepGraphError::InvalidParameter)?;
        let id = ExactBRepNodeId(
            self.nodes
                .len()
                .try_into()
                .map_err(|_: std::num::TryFromIntError| ExactBRepGraphError::ResourceLimit)?,
        );
        if self.nodes.len() >= MAX_EXACT_BREP_GRAPH_NODES {
            return Err(ExactBRepGraphError::ResourceLimit);
        }
        self.nodes.push(ExactBRepNode {
            id,
            source_feature_id: feature_id.0,
            operation: ExactBRepOperation::ThreadGroove(groove),
        });
        self.node_bounds.push(bounds);
        self.compiled_bodies.insert(feature_id, id);
        Ok(id)
    }

    fn modeled_thread(
        &self,
        feature_id: FeatureId,
    ) -> Result<Option<ModeledThread>, ExactBRepGraphError> {
        let unsupported = || ExactBRepGraphError::UnsupportedFeature(feature_id);
        if self.snapshot.hole_thread_is_modeled(feature_id) {
            let hole = self
                .snapshot
                .tapped_hole(feature_id)
                .ok_or_else(unsupported)?;
            let (start, direction) = (Vec3::from(hole.start_mm), Vec3::from(hole.direction));
            return Ok(Some(ModeledThread {
                metric: MetricThread {
                    nominal_diameter_mm: hole.nominal_diameter_mm,
                    pitch_mm: hole.pitch_mm,
                },
                phase: ThreadPhase::in_own_part(start, direction).ok_or_else(unsupported)?,
                start,
                end: start + direction * hole.length_mm,
                surface_diameter_mm: hole.surface_diameter_mm,
                internal: true,
            }));
        }
        let Some(shaft) = self.snapshot.shaft_thread(feature_id) else {
            return Ok(None);
        };
        let start = Vec3::from(shaft.start_mm);
        Ok(Some(ModeledThread {
            metric: MetricThread {
                nominal_diameter_mm: metric_nominal_diameter(&shaft.size)
                    .ok_or_else(unsupported)?,
                pitch_mm: shaft.pitch_mm,
            },
            phase: ThreadPhase {
                origin: shaft.phase_origin_mm.into(),
                axis: shaft.direction.into(),
                reference: shaft.phase_reference.into(),
            },
            start,
            end: start + Vec3::from(shaft.direction) * shaft.length_mm,
            surface_diameter_mm: shaft.surface_diameter_mm,
            internal: false,
        }))
    }
}

impl ModeledThread {
    /// The groove cut into `target`, whose body lies in `bounds`. At an end
    /// where the body ends too the groove runs a pitch past it, into empty
    /// space; where the body goes on (a bolt's head, the rest of a bore) it
    /// ends flat there. A bolt's shank is first turned down to its crest.
    fn groove(
        &self,
        target: ExactBRepNodeId,
        bounds: [[f64; 3]; 2],
        tolerance_mm: f64,
    ) -> Option<ExactBRepThreadGroove> {
        let pitch = self.metric.pitch_mm;
        let [a, b] = [
            self.phase.offset_of(self.start),
            self.phase.offset_of(self.end),
        ];
        let (low, high) = (a.min(b), a.max(b));
        let [body_low, body_high] = self.body_span(bounds);
        let ends = [
            if low <= body_low + tolerance_mm {
                low - pitch
            } else {
                low
            },
            if high >= body_high - tolerance_mm {
                high + pitch
            } else {
                high
            },
        ];
        let (profile, crest) = if self.internal {
            (self.metric.internal_groove(self.surface_diameter_mm)?, None)
        } else {
            let (crest, profile) = self.metric.turned_shank(self.surface_diameter_mm)?;
            (profile, Some(crest))
        };
        let runs_past = ends == [low - pitch, high + pitch];
        let trim = (crest.is_some() || !runs_past).then_some(ends);
        let [start, end] = match trim {
            Some([from, to]) => [from - pitch, to + pitch],
            None => ends,
        };
        let bits = |vector: Vec3| vector.to_array().map(canonical_bits);
        Some(ExactBRepThreadGroove {
            target,
            origin_bits: bits(self.phase.origin),
            axis_bits: bits(self.phase.axis),
            reference_bits: bits(self.phase.reference),
            pitch_bits: canonical_bits(pitch),
            start_bits: canonical_bits(start),
            end_bits: canonical_bits(end),
            left_handed: false,
            profile_bits: profile
                .iter()
                .map(|point| point.map(canonical_bits))
                .collect(),
            trim_bits: trim.map(|trim| trim.map(canonical_bits)),
            crest_bits: crest.map(|crest| crest.map(canonical_bits)),
        })
    }

    /// How far the box `bounds` reaches along the thread's axis, as offsets
    /// from its phase origin.
    fn body_span(&self, bounds: [[f64; 3]; 2]) -> [f64; 2] {
        let [low, high] = bounds;
        let mut span = [f64::INFINITY, f64::NEG_INFINITY];
        for corner in 0..8 {
            let point = Vec3::new(
                if corner & 1 == 0 { low[0] } else { high[0] },
                if corner & 2 == 0 { low[1] } else { high[1] },
                if corner & 4 == 0 { low[2] } else { high[2] },
            );
            let offset = self.phase.offset_of(point);
            span = [span[0].min(offset), span[1].max(offset)];
        }
        span
    }
}
