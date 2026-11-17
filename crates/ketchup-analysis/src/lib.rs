//! Engineering analysis over a Ketchup model: linear static FEA and hosted validator packages.
#![forbid(unsafe_code)]
// An error is carried, never dropped (review 2026-11-02 TST-121).
#![deny(clippy::let_underscore_must_use)]

pub mod fea;
mod fea_coupling;
mod fea_frame;
mod fea_modal;
mod sparse;
pub mod validator_hosting;
