//! Batch-scoped slots over the worker's digest-keyed graph outputs; pair results
//! are memoized by graph digests, placements and tolerance.
use super::*;
use crate::pair_query::{EXACT_PAIR_IDENTITY, MAX_EXACT_PAIR_CANDIDATES, MAX_EXACT_PAIR_GRAPHS};
use crate::protocol::{PairMeasure, PairQuery};

use std::rc::Rc;

/// Graph digests, both placements and tolerance fully determine a pair result.
type PairResultKey = (String, String, [u64; 16], [u64; 16], u64);
const MAX_CACHED_PAIR_RESULTS: usize = 100_000;

thread_local! {
    // Results are pure functions of their key, so they survive batch boundaries.
    static PAIR_RESULT_CACHE: std::cell::RefCell<BTreeMap<PairResultKey, PairMeasure>> =
        std::cell::RefCell::default();
}

pub(super) struct PairQuerySession {
    active: bool,
    bodies: Vec<(String, Rc<ketchup_exact::ExactOpOutput>)>,
    /// Placed copies of the batch's bodies, under a byte budget like the graph
    /// outputs; a copy that left is placed again when a query needs it.
    transformed: output_cache::OutputCache<ketchup_exact::ExactOpOutput>,
    queries: usize,
}

impl Default for PairQuerySession {
    fn default() -> Self {
        Self {
            active: false,
            bodies: Vec::new(),
            transformed: output_cache::OutputCache::new(
                MAX_EXACT_PAIR_CANDIDATES * 2,
                output_cache::MAX_CACHED_GRAPH_OUTPUT_BYTES,
            ),
            queries: 0,
        }
    }
}

impl PairQuerySession {
    pub(super) fn begin(&mut self) -> WorkerResult {
        *self = Self {
            active: true,
            ..Self::default()
        };
        Ok(WorkerReply::Done)
    }

    pub(super) fn end(&mut self) -> WorkerResult {
        let was_active = self.active;
        *self = Self::default();
        if was_active {
            Ok(WorkerReply::Done)
        } else {
            Err(WorkerFailure::invalid_request())
        }
    }

    pub(super) fn load(&mut self, backend: &ExactBackend, input: &GraphInput) -> WorkerResult {
        let result = self.try_load(backend, input);
        self.reset_on_failure(result)
    }

    /// A pair OCCT cannot measure fails alone: the batch stays loaded for the next pair.
    pub(super) fn query(&mut self, backend: &ExactBackend, query: &PairQuery) -> WorkerResult {
        let result = self.try_query(backend, query);
        if let Err(failure) = &result
            && crate::is_geometry_error_code(&failure.code)
        {
            self.queries += 1;
            return result;
        }
        self.reset_on_failure(result)
    }

    /// A failed batch is never resumed: the client restarts from `PairBegin`.
    fn reset_on_failure(&mut self, result: WorkerResult) -> WorkerResult {
        if result.is_err() {
            *self = Self::default();
        }
        result
    }

    fn try_load(&mut self, backend: &ExactBackend, input: &GraphInput) -> WorkerResult {
        let digest = &input.graph.graph_digest;
        if !self.active
            || self.queries != 0
            || self.bodies.len() >= MAX_EXACT_PAIR_GRAPHS
            || self.bodies.iter().any(|(existing, _)| existing == digest)
        {
            return Err(WorkerFailure::invalid_request());
        }
        let output = evaluated_graph_input(backend, input)?;
        let slot = self.bodies.len();
        self.bodies.push((digest.clone(), output));
        Ok(WorkerReply::PairLoaded { slot })
    }

    /// The body in `slot` placed by `matrix`.
    fn placed(
        &mut self,
        backend: &ExactBackend,
        slot: usize,
        matrix: [f64; 16],
    ) -> Result<Rc<ketchup_exact::ExactOpOutput>, WorkerFailure> {
        let Some((_, body)) = self.bodies.get(slot) else {
            return Err(WorkerFailure::invalid_request());
        };
        if matrix.iter().any(|value| !value.is_finite()) || matrix[12..] != [0.0, 0.0, 0.0, 1.0] {
            return Err(WorkerFailure::invalid_request());
        }
        if matrix == EXACT_PAIR_IDENTITY {
            return Ok(Rc::clone(body));
        }
        let key = format!("{slot}:{:?}", matrix.map(f64::to_bits));
        if let Some(placed) = self.transformed.get(&key) {
            return Ok(placed);
        }
        let placed = Rc::new(
            backend
                .transform_body(&body.body, &matrix)
                .map_err(|error| geometry_failure(&error))?,
        );
        let bytes = placed.retained_bytes();
        self.transformed.insert(key, Rc::clone(&placed), bytes);
        Ok(placed)
    }

    fn try_query(&mut self, backend: &ExactBackend, query: &PairQuery) -> WorkerResult {
        if !self.active
            || self.queries >= MAX_EXACT_PAIR_CANDIDATES
            || !query.tolerance_mm.is_finite()
            || query.tolerance_mm < 0.0
        {
            return Err(WorkerFailure::invalid_request());
        }
        let (Some((left_digest, _)), Some((right_digest, _))) =
            (self.bodies.get(query.left), self.bodies.get(query.right))
        else {
            return Err(WorkerFailure::invalid_request());
        };
        if let Some([left_face, right_face]) = query.faces {
            let distance_mm = backend
                .query_face_pair(
                    &self.bodies[query.left].1.body,
                    left_face,
                    &query.left_transform,
                    &self.bodies[query.right].1.body,
                    right_face,
                    &query.right_transform,
                )
                .map_err(|error| geometry_failure(&error))?;
            self.queries += 1;
            return Ok(WorkerReply::Pair(PairMeasure {
                common_volume_mm3: 0.0,
                common_thickness_mm: 0.0,
                common_contact_area_mm2: 0.0,
                contact_patches: Vec::new(),
                distance_mm,
            }));
        }
        let cache_key = (
            left_digest.clone(),
            right_digest.clone(),
            query.left_transform.map(f64::to_bits),
            query.right_transform.map(f64::to_bits),
            query.tolerance_mm.to_bits(),
        );
        let cached = PAIR_RESULT_CACHE.with(|cache| cache.borrow().get(&cache_key).cloned());
        let measure = match cached {
            Some(measure) => measure,
            None => {
                let left = self.placed(backend, query.left, query.left_transform)?;
                let right = self.placed(backend, query.right, query.right_transform)?;
                let result = backend
                    .query_body_pair(&left.body, &right.body, query.tolerance_mm)
                    .map_err(|error| geometry_failure(&error))?;
                let measure = PairMeasure {
                    common_volume_mm3: result.common_volume_mm3,
                    common_thickness_mm: result.common_thickness_mm,
                    common_contact_area_mm2: result.common_contact_area_mm2,
                    contact_patches: result.contact_patches,
                    distance_mm: result.distance_mm,
                };
                PAIR_RESULT_CACHE.with(|cache| {
                    let mut cache = cache.borrow_mut();
                    if cache.len() >= MAX_CACHED_PAIR_RESULTS {
                        cache.clear();
                    }
                    cache.insert(cache_key, measure.clone());
                });
                measure
            }
        };
        self.queries += 1;
        Ok(WorkerReply::Pair(measure))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Real v9 nightstand top board resting on a rotated side panel. The side's
    /// bearing face is a flat SurfaceOfExtrusion (not a Plane) and both optimal
    /// bounds carry tolerance, which once hid the bearing area entirely.
    #[test]
    fn modeled_board_on_rotated_side_panel_has_bearing_area_minus_dowel_holes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/v9_top_side_contact");
        let backend = ExactBackend::new();
        let load = |name: &str| {
            ExactBRepGraph::from_bytes(&std::fs::read(dir.join(name)).unwrap()).unwrap()
        };
        let (left_matrix, right_matrix): (Vec<f64>, Vec<f64>) =
            serde_json::from_slice(&std::fs::read(dir.join("matrices.json")).unwrap()).unwrap();
        let place = |graph: &str, matrix: Vec<f64>| {
            let body = evaluate_exact_brep_graph(&backend, &load(graph), &[]).unwrap();
            backend
                .transform_body(&body.body, &matrix.try_into().unwrap())
                .unwrap()
        };
        let top = place("left.graph", left_matrix);
        let side = place("right.graph", right_matrix);
        // 350 x 18 mm bearing face minus three D8 dowel holes.
        let expected = 350.0 * 18.0 - 3.0 * std::f64::consts::PI * 16.0;
        for (left, right) in [(&top, &side), (&side, &top)] {
            let result = backend
                .query_body_pair(&left.body, &right.body, 1e-7)
                .unwrap();
            assert_eq!(result.relation, ketchup_exact::ExactPairRelation::Touching);
            assert_eq!(result.common_volume_mm3, 0.0);
            assert_eq!(result.distance_mm, 0.0);
            assert!(
                (result.common_contact_area_mm2 - expected).abs() < 1e-6,
                "{result:?}"
            );
        }
    }
    #[test]
    fn query_session_rejects_missing_bodies_and_clears_failed_batch() {
        let backend = ExactBackend::new();
        let mut session = PairQuerySession::default();
        assert_eq!(session.begin(), Ok(WorkerReply::Done));
        let query = PairQuery {
            left: 0,
            right: 1,
            tolerance_mm: 0.0,
            faces: None,
            left_transform: EXACT_PAIR_IDENTITY,
            right_transform: EXACT_PAIR_IDENTITY,
        };
        assert_eq!(
            session.query(&backend, &query),
            Err(WorkerFailure::invalid_request())
        );
        assert!(!session.active);
        assert!(session.bodies.is_empty());
        assert!(session.transformed.keys().is_empty());
        assert_eq!(session.end(), Err(WorkerFailure::invalid_request()));
    }

    #[test]
    fn transformed_native_bodies_are_reused_and_batch_scoped() {
        let backend = ExactBackend::new();
        let mut session = PairQuerySession {
            active: true,
            ..PairQuerySession::default()
        };
        let shape = backend
            .extrude_circle(ketchup_exact::CircleExtrudeSpec {
                center_mm: [0.0, 0.0],
                radius_mm: 1.0,
                height_mm: 2.0,
            })
            .unwrap();
        session.bodies.push(("fixture".to_owned(), Rc::new(shape)));
        let mut matrix = EXACT_PAIR_IDENTITY;
        matrix[3] = 3.0;
        let first = session.placed(&backend, 0, matrix).unwrap();
        for _ in 0..10 {
            assert!(Rc::ptr_eq(
                &session.placed(&backend, 0, matrix).unwrap(),
                &first
            ));
        }
        assert_eq!(session.transformed.keys().len(), 1);
        matrix[3] = 4.0;
        assert!(!Rc::ptr_eq(
            &session.placed(&backend, 0, matrix).unwrap(),
            &first
        ));
        assert_eq!(session.transformed.keys().len(), 2);
        let unplaced = session.placed(&backend, 0, EXACT_PAIR_IDENTITY).unwrap();
        assert!(Rc::ptr_eq(&unplaced, &session.bodies[0].1));
        matrix[0] = 0.0;
        assert!(session.placed(&backend, 0, matrix).is_err());
        assert_eq!(session.end(), Ok(WorkerReply::Done));
        assert!(session.transformed.keys().is_empty());
        assert!(session.bodies.is_empty());
    }

    #[test]
    fn placed_copies_stay_within_their_byte_budget() {
        let backend = ExactBackend::new();
        let shape = backend
            .extrude_circle(ketchup_exact::CircleExtrudeSpec {
                center_mm: [0.0, 0.0],
                radius_mm: 1.0,
                height_mm: 2.0,
            })
            .unwrap();
        // Room for one placed copy of this body, not two.
        let budget = shape.retained_bytes() * 3 / 2;
        let mut session = PairQuerySession {
            active: true,
            transformed: output_cache::OutputCache::new(MAX_EXACT_PAIR_CANDIDATES * 2, budget),
            ..PairQuerySession::default()
        };
        session.bodies.push(("fixture".to_owned(), Rc::new(shape)));
        let mut matrix = EXACT_PAIR_IDENTITY;
        for step in 1..=5 {
            matrix[3] = f64::from(step);
            session.placed(&backend, 0, matrix).unwrap();
            assert_eq!(session.transformed.keys().len(), 1);
        }
        // The copy that left is placed again, not refused.
        matrix[3] = 1.0;
        let again = session.placed(&backend, 0, matrix).unwrap();
        assert!(!again.body.topology.faces.is_empty());
        assert_eq!(
            session.transformed.keys(),
            [format!("0:{:?}", matrix.map(f64::to_bits))]
        );
    }
}
