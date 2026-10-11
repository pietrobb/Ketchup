# starlark 0.13.0, patched for Ketchup

Unmodified copy of `starlark` 0.13.0 from crates.io except two changes. First, in
`src/eval/bc/writer.rs` (`BcWriter::write_for`): the first instruction of every loop
iteration is recorded as a statement location, so the `before_stmt` hook runs once per
iteration.

Second change, a heap bound (`Heap::set_allocation_limit`, `src/values/layout/heap/heap_type.rs`):
`str * n`, `list * n`, `tuple * n`, `str + str`, `list + list`, `list(iterable)`,
`tuple(iterable)` and `str.join` check the bytes they would build against the bound and
fail with "program memory limit exceeded" before allocating. Evaluation runs inside the
Kečup window process, and one such line (`"a" * 1500000000`) took gigabytes before the
`before_stmt` hook could see it. The hook checks the whole heap after every statement.

`src/lib.rs` also allows two lints that newer rustc reports in the unchanged upstream
code (`deprecated`, `mismatched_lifetime_syntaxes`), so the build stays quiet.

Why: `ketchup-program` bounds program evaluation (`execution_budget.rs`) through
`before_stmt`. Upstream emits the hook only before statements, so a comprehension
(`[0 for a in L for b in L]`) or a loop whose body is only `pass` iterated without
reaching it and could run without limit.

Remove this copy and the `[patch.crates-io]` entry in the root `Cargo.toml` once an
upstream release counts loop iterations and the workspace can move to it (0.14 turns on
serde_json `arbitrary_precision`, see `crates/ketchup-program/Cargo.toml`).
