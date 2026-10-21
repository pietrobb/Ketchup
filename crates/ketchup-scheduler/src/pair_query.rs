//! Read-only, batch-scoped exact collision queries. No result is canonical state.
use super::*;
use crate::protocol::{GraphInput, PairMeasure, PairQuery, WorkerReply, WorkerRequest};
pub use ketchup_exact::{ExactPairQueryResult, ExactPairRelation};
use ketchup_geometry::contact_patch::ContactPatch;

pub const MAX_EXACT_PAIR_GRAPHS: usize = 512;
pub const MAX_EXACT_PAIR_CANDIDATES: usize = 10_000;
pub const EXACT_PAIR_IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// Indices into the supplied graphs and row-major local-to-world affine matrices.
/// Translations occupy indices 3, 7, 11. Both matrices use the same world frame.
#[derive(Clone, Debug, PartialEq)]
pub struct ExactPairCandidate {
    pub left_graph: usize,
    pub right_graph: usize,
    pub left_transform: [f64; 16],
    pub right_transform: [f64; 16],
}

/// One candidate's answer: its exact relation, or the geometry error OCCT gave
/// for that pair alone. An error is never separated evidence.
pub type ExactPairAnswer = Result<ExactPairQueryResult, WorkerError>;

impl ExactWorkerSupervisor {
    /// Queries candidates in input order with one evaluation per unique graph digest.
    /// Imported sources are SHA-256-keyed original STEP bytes (an empty map for modeled graphs).
    /// A pair OCCT cannot measure gets its own `Err` answer and the batch goes on;
    /// any other failure returns Err for the whole batch, never partial evidence.
    /// No restart/retry occurs within a batch. Native cache is released on success;
    /// on failure the worker is terminated and respawned before a subsequent batch.
    /// Empty candidates return an empty vector. Sources are staged once per digest.
    pub fn query_exact_brep_pairs(
        &mut self,
        graphs: &[ExactBRepGraph],
        candidates: &[ExactPairCandidate],
        imported_source_blobs: &BTreeMap<String, Vec<u8>>,
        contact_tolerance_mm: f64,
    ) -> Result<Vec<ExactPairAnswer>, WorkerError> {
        self.query_exact_brep_pairs_with_cancellation(
            graphs,
            candidates,
            imported_source_blobs,
            contact_tolerance_mm,
            &NEVER_CANCELLED,
        )
    }

    pub fn query_exact_brep_pairs_with_cancellation(
        &mut self,
        graphs: &[ExactBRepGraph],
        candidates: &[ExactPairCandidate],
        imported_source_blobs: &BTreeMap<String, Vec<u8>>,
        contact_tolerance_mm: f64,
        cancelled: &AtomicBool,
    ) -> Result<Vec<ExactPairAnswer>, WorkerError> {
        self.query_pairs(
            graphs,
            candidates,
            imported_source_blobs,
            contact_tolerance_mm,
            cancelled,
            None,
        )
    }

    /// One bounded selected-face query using the same isolated graph loading and timeout path.
    pub fn query_exact_brep_faces_with_cancellation(
        &mut self,
        graphs: &[ExactBRepGraph],
        candidate: &ExactPairCandidate,
        faces: [u32; 2],
        imported_source_blobs: &BTreeMap<String, Vec<u8>>,
        cancelled: &AtomicBool,
    ) -> Result<f64, WorkerError> {
        let results = self.query_pairs(
            graphs,
            std::slice::from_ref(candidate),
            imported_source_blobs,
            0.0,
            cancelled,
            Some(faces),
        )?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| WorkerError::Protocol("missing exact face distance".to_owned()))?
            .map(|result| result.distance_mm)
    }

    fn query_pairs(
        &mut self,
        graphs: &[ExactBRepGraph],
        candidates: &[ExactPairCandidate],
        imported_source_blobs: &BTreeMap<String, Vec<u8>>,
        contact_tolerance_mm: f64,
        cancelled: &AtomicBool,
        faces: Option<[u32; 2]>,
    ) -> Result<Vec<ExactPairAnswer>, WorkerError> {
        let invalid = || WorkerError::Protocol("invalid or oversized exact pair batch".to_owned());
        check_pair_cancelled(cancelled)?;
        if candidates.len() > MAX_EXACT_PAIR_CANDIDATES
            || !contact_tolerance_mm.is_finite()
            || contact_tolerance_mm < 0.0
            || candidates.iter().any(|pair| {
                pair.left_graph >= graphs.len()
                    || pair.right_graph >= graphs.len()
                    || !valid_pair_transform(&pair.left_transform)
                    || !valid_pair_transform(&pair.right_transform)
            })
        {
            return Err(invalid());
        }
        if candidates.is_empty() {
            return Ok(Vec::new());
        }
        // Validate serialized identity, not merely the caller's public digest field.
        let mut unique = BTreeMap::<String, usize>::new();
        let mut indices = BTreeMap::new();
        let mut prepared = Vec::new();
        let mut staged = PairSources::default();
        for index in candidates
            .iter()
            .flat_map(|pair| [pair.left_graph, pair.right_graph])
        {
            check_pair_cancelled(cancelled)?;
            if indices.contains_key(&index) {
                continue;
            }
            let graph = &graphs[index];
            let bytes = graph
                .to_bytes()
                .map_err(|error| WorkerError::Protocol(error.to_string()))?;
            check_pair_cancelled(cancelled)?;
            let verified = ExactBRepGraph::from_bytes(&bytes)
                .map_err(|error| WorkerError::Protocol(error.to_string()))?;
            if verified.graph_digest != graph.graph_digest {
                return Err(invalid());
            }
            // The digest covers all the worker evaluates; another document's copy reuses the slot.
            if let Some(slot) = unique.get(&graph.graph_digest) {
                indices.insert(index, *slot);
                continue;
            }
            if prepared.len() >= MAX_EXACT_PAIR_GRAPHS {
                return Err(invalid());
            }
            let sources = staged.prepare_graph(graph, imported_source_blobs, cancelled)?;
            let slot = prepared.len();
            unique.insert(graph.graph_digest.clone(), slot);
            indices.insert(index, slot);
            prepared.push((graph, sources));
        }
        check_pair_cancelled(cancelled)?;
        // Recover only at the batch boundary. A failed batch is never replayed.
        if self.client.terminated
            || self
                .client
                .child
                .try_wait()
                .map_err(|error| WorkerError::Transport(error.to_string()))?
                .is_some()
        {
            self.client =
                Self::spawn_verified_client(&self.executable, &self.executable_sha256, cancelled)?;
        }
        let result = (|| {
            self.client.call_done(
                &WorkerRequest::PairBegin,
                cancelled,
                DEFAULT_WORKER_REQUEST_TIMEOUT,
            )?;
            for (slot, (graph, sources)) in prepared.iter().enumerate() {
                let input = GraphInput {
                    graph: (*graph).clone(),
                    sources: sources
                        .iter()
                        .map(|hash| worker_client::source_file(hash, staged.files[hash].1.path()))
                        .collect(),
                };
                match self.client.call(
                    &WorkerRequest::PairLoad(input),
                    cancelled,
                    EXACT_BREP_GRAPH_REQUEST_TIMEOUT,
                )? {
                    WorkerReply::PairLoaded { slot: loaded } if loaded == slot => {}
                    reply => return Err(worker_client::unexpected_reply(&reply)),
                }
            }
            let mut results = Vec::with_capacity(candidates.len());
            for candidate in candidates {
                let query = PairQuery {
                    left: indices[&candidate.left_graph],
                    right: indices[&candidate.right_graph],
                    tolerance_mm: contact_tolerance_mm,
                    faces,
                    left_transform: candidate.left_transform,
                    right_transform: candidate.right_transform,
                };
                match self.client.call(
                    &WorkerRequest::PairQuery(query),
                    cancelled,
                    EXACT_BREP_GRAPH_REQUEST_TIMEOUT,
                ) {
                    Ok(WorkerReply::Pair(measure)) => {
                        results.push(Ok(pair_result(measure, contact_tolerance_mm)?));
                    }
                    // The worker keeps the batch loaded after a geometry refusal.
                    Err(error @ WorkerError::Geometry(_)) => results.push(Err(error)),
                    Ok(reply) => return Err(worker_client::unexpected_reply(&reply)),
                    Err(error) => return Err(error),
                }
            }
            self.client.call_done(
                &WorkerRequest::PairEnd,
                cancelled,
                DEFAULT_WORKER_REQUEST_TIMEOUT,
            )?;
            Ok(results)
        })();
        if result.is_err() {
            self.client.terminate_worker();
        }
        result
    }
}

fn check_pair_cancelled(cancelled: &AtomicBool) -> Result<(), WorkerError> {
    if cancelled.load(Ordering::Acquire) {
        Err(WorkerError::Cancelled)
    } else {
        Ok(())
    }
}

#[derive(Default)]
struct PairSources {
    files: BTreeMap<String, (u64, tempfile::NamedTempFile)>,
}

impl PairSources {
    fn prepare_graph(
        &mut self,
        graph: &ExactBRepGraph,
        blobs: &BTreeMap<String, Vec<u8>>,
        cancelled: &AtomicBool,
    ) -> Result<Vec<String>, WorkerError> {
        let mut sources = Vec::new();
        for node in &graph.nodes {
            check_pair_cancelled(cancelled)?;
            let ExactBRepOperation::ImportedExact {
                source_sha256,
                source_byte_len,
                ..
            } = &node.operation
            else {
                continue;
            };
            let hash = hex_encode(source_sha256);
            self.stage(&hash, *source_byte_len, blobs, cancelled)?;
            if !sources.contains(&hash) {
                if sources.len() >= MAX_EXACT_BREP_GRAPH_IMPORTED_SOURCES {
                    return Err(WorkerError::Protocol(
                        "too many exact pair graph sources".to_owned(),
                    ));
                }
                sources.push(hash);
            }
        }
        Ok(sources)
    }

    fn stage(
        &mut self,
        hash: &str,
        expected_len: u64,
        blobs: &BTreeMap<String, Vec<u8>>,
        cancelled: &AtomicBool,
    ) -> Result<(), WorkerError> {
        check_pair_cancelled(cancelled)?;
        let invalid = || WorkerError::Protocol("invalid exact pair sources".to_owned());
        if let Some((previous_len, _)) = self.files.get(hash) {
            return if *previous_len == expected_len {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        let source = blobs.get(hash).ok_or_else(invalid)?;
        if source.len() as u64 != expected_len {
            return Err(invalid());
        }
        // Hash once per distinct source, with cancellation on both sides of the
        // bounded hash operation; do not duplicate bytes just to verify identity.
        if sha256_hex(source) != hash {
            return Err(invalid());
        }
        check_pair_cancelled(cancelled)?;
        let mut file = tempfile::Builder::new()
            .prefix(".ketchup-pair-source-")
            .suffix(".step")
            .tempfile()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        write_pair_source(&mut file, source, cancelled)?;
        self.files.insert(hash.to_owned(), (expected_len, file));
        Ok(())
    }
}

fn write_pair_source(
    writer: &mut impl std::io::Write,
    source: &[u8],
    cancelled: &AtomicBool,
) -> Result<(), WorkerError> {
    for chunk in source.chunks(64 * 1024) {
        check_pair_cancelled(cancelled)?;
        writer
            .write_all(chunk)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
    }
    check_pair_cancelled(cancelled)?;
    writer
        .flush()
        .map_err(|error| WorkerError::Transport(error.to_string()))?;
    check_pair_cancelled(cancelled)
}

fn valid_pair_transform(matrix: &[f64; 16]) -> bool {
    matrix.iter().all(|v| v.is_finite()) && matrix[12..] == [0.0, 0.0, 0.0, 1.0]
}

fn pair_result(measure: PairMeasure, tolerance: f64) -> Result<ExactPairQueryResult, WorkerError> {
    let PairMeasure {
        common_volume_mm3,
        common_thickness_mm,
        common_contact_area_mm2,
        ref contact_patches,
        distance_mm,
    } = measure;
    if [
        common_volume_mm3,
        common_thickness_mm,
        common_contact_area_mm2,
        distance_mm,
    ]
    .iter()
    .any(|value| !value.is_finite() || *value < 0.0)
        || (common_volume_mm3 > 0.0) != (common_thickness_mm > 0.0)
        || (common_volume_mm3 > 0.0 && (common_contact_area_mm2 > 0.0 || distance_mm != 0.0))
        || (common_contact_area_mm2 > 0.0 && distance_mm != 0.0)
        || !ContactPatch::consistent(contact_patches, common_contact_area_mm2)
    {
        return Err(WorkerError::Protocol(format!(
            "invalid exact pair evidence {measure:?}"
        )));
    }
    Ok(ExactPairQueryResult {
        relation: if common_volume_mm3 > 0.0 {
            ExactPairRelation::Penetrating
        } else if distance_mm <= tolerance {
            ExactPairRelation::Touching
        } else {
            ExactPairRelation::Separated
        },
        common_volume_mm3,
        common_thickness_mm,
        common_contact_area_mm2,
        contact_patches: measure.contact_patches,
        distance_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_sources_deduplicate_bound_verify_and_clean_up() {
        let source = b"source".to_vec();
        let hash = sha256_hex(&source);
        let blobs = BTreeMap::from([(hash.clone(), source)]);
        let mut staged = PairSources::default();
        staged.stage(&hash, 6, &blobs, &NEVER_CANCELLED).unwrap();
        let path = staged.files[&hash].1.path().to_owned();
        for _ in 0..MAX_EXACT_PAIR_GRAPHS {
            staged.stage(&hash, 6, &blobs, &NEVER_CANCELLED).unwrap();
        }
        assert_eq!(staged.files.len(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), blobs[&hash]);
        assert!(staged.stage(&hash, 7, &blobs, &NEVER_CANCELLED).is_err());
        assert!(matches!(
            staged.stage(&hash, 6, &blobs, &AtomicBool::new(true)),
            Err(WorkerError::Cancelled)
        ));
        drop(staged);
        assert!(!path.exists());

        // A source past the 128 MiB the whole batch was once held to stages.
        let large = vec![b'x'; 128 * 1024 * 1024 + 1];
        let large_hash = sha256_hex(&large);
        let large_len = large.len() as u64;
        let large_blobs = BTreeMap::from([(large_hash.clone(), large)]);
        let mut staged = PairSources::default();
        staged.stage(&hash, 6, &blobs, &NEVER_CANCELLED).unwrap();
        staged
            .stage(&large_hash, large_len, &large_blobs, &NEVER_CANCELLED)
            .unwrap();
        assert_eq!(staged.files.len(), 2);
        assert_eq!(
            std::fs::metadata(staged.files[&large_hash].1.path())
                .unwrap()
                .len(),
            large_len
        );
        drop(staged);
        let mut staged = PairSources::default();
        let corrupt = BTreeMap::from([(hash.clone(), b"broken".to_vec())]);
        assert!(staged.stage(&hash, 6, &corrupt, &NEVER_CANCELLED).is_err());
        assert!(
            staged
                .stage(&hash, 6, &BTreeMap::new(), &NEVER_CANCELLED)
                .is_err()
        );
        assert!(
            staged
                .stage(&hash, u64::MAX, &blobs, &NEVER_CANCELLED)
                .is_err()
        );
        assert!(staged.files.is_empty());
    }

    #[test]
    fn pair_staging_cancels_between_write_chunks_and_after_last_write() {
        struct CancelWriter<'a> {
            cancelled: &'a AtomicBool,
            bytes: usize,
        }
        impl std::io::Write for CancelWriter<'_> {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes += bytes.len();
                self.cancelled.store(true, Ordering::Release);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                panic!("must not flush after cancellation")
            }
        }
        for size in [64 * 1024, 128 * 1024] {
            let cancelled = AtomicBool::new(false);
            let mut writer = CancelWriter {
                cancelled: &cancelled,
                bytes: 0,
            };
            assert!(matches!(
                write_pair_source(&mut writer, &vec![0; size], &cancelled),
                Err(WorkerError::Cancelled)
            ));
            assert_eq!(writer.bytes, 64 * 1024);
        }
    }

    #[test]
    fn pair_protocol_never_converts_invalid_evidence_to_separated() {
        let measure = |common_volume_mm3: f64, common_contact_area_mm2, distance_mm| PairMeasure {
            common_volume_mm3,
            common_thickness_mm: common_volume_mm3.min(1.0),
            common_contact_area_mm2,
            contact_patches: Vec::new(),
            distance_mm,
        };
        let patch = |area_mm2| ContactPatch {
            normal: [0.0, 0.0, -1.0],
            area_mm2,
        };
        for invalid in [
            // Shared area must say which way it faces, and add up.
            measure(0.0, 2.0, 0.0),
            PairMeasure {
                contact_patches: vec![patch(1.0)],
                ..measure(0.0, 2.0, 0.0)
            },
            PairMeasure {
                common_thickness_mm: 0.0,
                ..measure(1.0, 0.0, 0.0)
            },
            PairMeasure {
                common_thickness_mm: 1.0,
                ..measure(0.0, 0.0, 0.0)
            },
            measure(f64::NAN, 0.0, 0.0),
            measure(1.0, 1.0, 0.0),
            measure(1.0, 0.0, 1.0),
            measure(0.0, 1.0, 1.0),
            measure(0.0, -1.0, 0.0),
            measure(0.0, 0.0, f64::INFINITY),
        ] {
            assert!(pair_result(invalid.clone(), 1e-7).is_err(), "{invalid:?}");
        }
        let touching = PairMeasure {
            contact_patches: vec![patch(2.0)],
            ..measure(0.0, 2.0, 0.0)
        };
        let touching = pair_result(touching, 1e-7).unwrap();
        assert_eq!(touching.relation, ExactPairRelation::Touching);
        assert_eq!(touching.contact_patches, vec![patch(2.0)]);
        assert_eq!(
            pair_result(measure(0.0, 0.0, 2.0), 1e-7).unwrap().relation,
            ExactPairRelation::Separated
        );
    }
}
