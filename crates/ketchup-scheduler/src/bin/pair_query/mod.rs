//! Batch-scoped slots over the worker's digest-keyed graph outputs; pair results
//! are memoized by graph digests, placements and tolerance.
use super::*;
use crate::pair_query::{EXACT_PAIR_IDENTITY, MAX_EXACT_PAIR_CANDIDATES, MAX_EXACT_PAIR_GRAPHS};

type TransformKey = (usize, [u64; 16]);
/// Graph digests, both placements and tolerance fully determine a pair result.
type PairResultKey = (String, String, [u64; 16], [u64; 16], u64);
const MAX_CACHED_PAIR_RESULTS: usize = 100_000;

thread_local! {
    // Results are pure functions of their key, so they survive batch boundaries.
    static PAIR_RESULT_CACHE: std::cell::RefCell<BTreeMap<PairResultKey, [u64; 3]>> =
        std::cell::RefCell::default();
}

#[derive(Default)]
pub(super) struct PairQuerySession {
    active: bool,
    bodies: Vec<(String, std::rc::Rc<ketchup_exact::ExactOpOutput>)>,
    transformed: BTreeMap<TransformKey, ketchup_exact::ExactBody>,
    queries: usize,
}

impl PairQuerySession {
    pub(super) fn handle(&mut self, backend: &ExactBackend, request: &str) -> Option<String> {
        let fields = request.split_whitespace().collect::<Vec<_>>();
        if fields.as_slice() == ["CAPS", "EXACT_PAIR_V2"] {
            return Some("CAPS EXACT_PAIR_V2".to_owned());
        }
        if !fields
            .first()
            .is_some_and(|field| field.starts_with("PAIR_"))
        {
            return None;
        }
        let response = self.execute(backend, request.trim(), &fields);
        if response.starts_with("ERR") {
            *self = Self::default();
        }
        Some(response)
    }

    fn prepare_transform(
        &mut self,
        backend: &ExactBackend,
        slot: usize,
        matrix: [f64; 16],
    ) -> Result<Option<TransformKey>, String> {
        let Some((_, body)) = self.bodies.get(slot) else {
            return Err("ERR invalid_request".to_owned());
        };
        if matrix.iter().any(|value| !value.is_finite()) || matrix[12..] != [0.0, 0.0, 0.0, 1.0] {
            return Err("ERR invalid_request".to_owned());
        }
        if matrix == EXACT_PAIR_IDENTITY {
            return Ok(None);
        }
        let key = (slot, matrix.map(f64::to_bits));
        if !self.transformed.contains_key(&key) {
            if self.transformed.len() >= MAX_EXACT_PAIR_CANDIDATES * 2 {
                return Err("ERR invalid_request".to_owned());
            }
            let output = backend
                .transform_body(&body.body, &matrix)
                .map_err(|error| geometry_error_response(&error))?;
            self.transformed.insert(key, output.body);
        }
        Ok(Some(key))
    }

    fn body(&self, slot: usize, key: Option<TransformKey>) -> &ketchup_exact::ExactBody {
        key.map_or_else(|| &self.bodies[slot].1.body, |key| &self.transformed[&key])
    }

    fn execute(&mut self, backend: &ExactBackend, request: &str, fields: &[&str]) -> String {
        match fields {
            ["PAIR_BEGIN_V1"] => {
                *self = Self {
                    active: true,
                    ..Self::default()
                };
                "OK_PAIR_BEGIN_V1".to_owned()
            }
            ["PAIR_END_V1"] if self.active => {
                *self = Self::default();
                "OK_PAIR_END_V1".to_owned()
            }
            ["PAIR_LOAD_V1", digest, encoded, sources @ ..] if self.active && self.queries == 0 => {
                if self.bodies.len() >= MAX_EXACT_PAIR_GRAPHS
                    || self.bodies.iter().any(|(existing, _)| existing == digest)
                {
                    return "ERR invalid_request".to_owned();
                }
                let Some(bytes) = decode_hex_bytes(encoded) else {
                    return "ERR invalid_request".to_owned();
                };
                let Ok(graph) = ExactBRepGraph::from_bytes(&bytes) else {
                    return "ERR invalid_request".to_owned();
                };
                if graph.graph_digest != *digest {
                    return "ERR invalid_request".to_owned();
                }
                let sources = match verified_exact_brep_graph_sources(&graph, sources) {
                    Ok(sources) => sources,
                    Err(response) => return response,
                };
                let output = match evaluate_exact_brep_graph_cached(backend, &graph, &sources) {
                    Ok(output) => output,
                    Err(error) => return geometry_error_response(&error),
                };
                let slot = self.bodies.len();
                self.bodies.push((graph.graph_digest, output));
                format!("OK_PAIR_LOAD_V1 {slot} {digest}")
            }
            ["PAIR_QUERY_V2", left, right, tolerance, matrices @ ..]
                if self.active
                    && matrices.len() == 32
                    && self.queries < MAX_EXACT_PAIR_CANDIDATES =>
            {
                let (Ok(left), Ok(right)) = (left.parse::<usize>(), right.parse::<usize>()) else {
                    return "ERR invalid_request".to_owned();
                };
                let decode = |s: &str| {
                    u64::from_str_radix(s, 16)
                        .ok()
                        .filter(|_| s.len() == 16)
                        .map(f64::from_bits)
                        .filter(|v| v.is_finite())
                };
                let Some(tolerance) = decode(tolerance).filter(|v| *v >= 0.0) else {
                    return "ERR invalid_request".to_owned();
                };
                let Some(values) = matrices
                    .iter()
                    .map(|s| decode(s))
                    .collect::<Option<Vec<_>>>()
                else {
                    return "ERR invalid_request".to_owned();
                };
                let (Some((left_digest, _)), Some((right_digest, _))) =
                    (self.bodies.get(left), self.bodies.get(right))
                else {
                    return "ERR invalid_request".to_owned();
                };
                let left_matrix: [f64; 16] = values[..16].try_into().expect("matrix length");
                let right_matrix: [f64; 16] = values[16..].try_into().expect("matrix length");
                let cache_key = (
                    left_digest.clone(),
                    right_digest.clone(),
                    left_matrix.map(f64::to_bits),
                    right_matrix.map(f64::to_bits),
                    tolerance.to_bits(),
                );
                let cached =
                    PAIR_RESULT_CACHE.with(|cache| cache.borrow().get(&cache_key).copied());
                let bits = match cached {
                    Some(bits) => bits,
                    None => {
                        let left_key = match self.prepare_transform(backend, left, left_matrix) {
                            Ok(key) => key,
                            Err(error) => return error,
                        };
                        let right_key = match self.prepare_transform(backend, right, right_matrix) {
                            Ok(key) => key,
                            Err(error) => return error,
                        };
                        match backend.query_body_pair(
                            self.body(left, left_key),
                            self.body(right, right_key),
                            tolerance,
                        ) {
                            Ok(result) => {
                                let bits = [
                                    result.common_volume_mm3.to_bits(),
                                    result.common_contact_area_mm2.to_bits(),
                                    result.distance_mm.to_bits(),
                                ];
                                PAIR_RESULT_CACHE.with(|cache| {
                                    let mut cache = cache.borrow_mut();
                                    if cache.len() >= MAX_CACHED_PAIR_RESULTS {
                                        cache.clear();
                                    }
                                    cache.insert(cache_key, bits);
                                });
                                bits
                            }
                            Err(error) => return geometry_error_response(&error),
                        }
                    }
                };
                self.queries += 1;
                format!(
                    "OK_PAIR_QUERY_V2 {} {:016x} {:016x} {:016x}",
                    sha256_hex(request.as_bytes()),
                    bits[0],
                    bits[1],
                    bits[2]
                )
            }
            _ => "ERR invalid_request".to_owned(),
        }
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
        assert_eq!(
            session.handle(&backend, "PAIR_BEGIN_V1").unwrap(),
            "OK_PAIR_BEGIN_V1"
        );
        assert!(
            session
                .handle(&backend, "PAIR_LOAD_V1 bad bad")
                .unwrap()
                .starts_with("ERR")
        );
        assert!(!session.active);
        assert!(session.bodies.is_empty());
        assert!(session.transformed.is_empty());
        assert!(
            session
                .handle(&backend, "PAIR_END_V1")
                .unwrap()
                .starts_with("ERR")
        );
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
        session
            .bodies
            .push(("fixture".to_owned(), std::rc::Rc::new(shape)));
        let mut matrix = EXACT_PAIR_IDENTITY;
        matrix[3] = 3.0;
        let key = session.prepare_transform(&backend, 0, matrix).unwrap();
        let address = session.body(0, key) as *const ketchup_exact::ExactBody;
        for _ in 0..10 {
            assert_eq!(session.prepare_transform(&backend, 0, matrix).unwrap(), key);
            assert_eq!(
                session.body(0, key) as *const ketchup_exact::ExactBody,
                address
            );
        }
        assert_eq!(session.transformed.len(), 1);
        matrix[3] = 4.0;
        assert_ne!(session.prepare_transform(&backend, 0, matrix).unwrap(), key);
        assert_eq!(session.transformed.len(), 2);
        assert_eq!(
            session
                .prepare_transform(&backend, 0, EXACT_PAIR_IDENTITY)
                .unwrap(),
            None
        );
        matrix[0] = 0.0;
        assert!(session.prepare_transform(&backend, 0, matrix).is_err());
        assert_eq!(
            session.handle(&backend, "PAIR_END_V1").unwrap(),
            "OK_PAIR_END_V1"
        );
        assert!(session.transformed.is_empty());
        assert!(session.bodies.is_empty());
    }
}
