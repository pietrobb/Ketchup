//! Worker failures built from OCCT errors.

use crate::protocol::{FailureDetail, WorkerFailure};
use ketchup_model::graph::sha256_hex;

pub(super) fn geometry_failure(error: &ketchup_exact::GeometryError) -> WorkerFailure {
    WorkerFailure {
        code: error.code.as_str().to_owned(),
        detail: Some(FailureDetail {
            diagnostic: error.diagnostic.clone(),
            operation: error.operation.to_owned(),
            input_digest: error.input_digest.clone(),
            backend: error.backend_fingerprint.to_owned(),
        }),
    }
}

pub(super) fn backend_failure(operation: &str, diagnostic: &str) -> WorkerFailure {
    WorkerFailure {
        code: ketchup_exact::GeometryErrorCode::BackendException
            .as_str()
            .to_owned(),
        detail: Some(FailureDetail {
            diagnostic: diagnostic.to_owned(),
            operation: operation.to_owned(),
            input_digest: sha256_hex(diagnostic.as_bytes()),
            backend: ketchup_exact::backend_fingerprint().to_owned(),
        }),
    }
}
