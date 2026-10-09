# starlark 0.13.0, patched for Ketchup

Unmodified copy of `starlark` 0.13.0 from crates.io except one change in
`src/eval/bc/writer.rs` (`BcWriter::write_for`): the first instruction of every loop
iteration is recorded as a statement location, so the `before_stmt` hook runs once per
iteration.

`src/lib.rs` also allows two lints that newer rustc reports in the unchanged upstream
code (`deprecated`, `mismatched_lifetime_syntaxes`), so the build stays quiet.

Why: `ketchup-program` bounds program evaluation (`execution_budget.rs`) through
`before_stmt`. Upstream emits the hook only before statements, so a comprehension
(`[0 for a in L for b in L]`) or a loop whose body is only `pass` iterated without
reaching it and could run without limit.

Remove this copy and the `[patch.crates-io]` entry in the root `Cargo.toml` once an
upstream release counts loop iterations and the workspace can move to it (0.14 turns on
serde_json `arbitrary_precision`, see `crates/ketchup-program/Cargo.toml`).
