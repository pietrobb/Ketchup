//! Local product data management: signed releases of Ketchup documents and their dependencies in a content-addressed repository.
#![forbid(unsafe_code)]
// An error is carried, never dropped (review 2026-11-02 TST-121).
#![deny(clippy::let_underscore_must_use)]

pub mod local;
