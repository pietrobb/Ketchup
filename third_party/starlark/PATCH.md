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

The bound also covers operations that build their result outside the heap first (review
2026-10-15 PRG-6): `str.replace` computes the size of its result before building it;
`str.format`, `%`, `str()`, `repr()` and `json.encode` check the text as it grows, and a
container `repr` stops adding elements once the text passes the bound of the heap last
limited on the thread (`repr_limit_reached`). Big integers count their digits against the
bound (`Heap::bounded_bytes`), and a product past 2^20 bits fails like upstream's left
shift past 100 000 bits, since one such multiplication cannot be interrupted. A garbage
collection counts the digits of the big integers that survive it again, and it is due by
`bounded_bytes` instead of the arena alone (`src/eval/compiler/stmt.rs`), so replaced big
integers no longer count until the program reaches the bound (review 2026-10-15 status PRG-6).

`Heap::memory_limit_reached` tells whether the bound refused an operation or the heap holds
more than it. The refusal's error loses its type when a method returns it as
`anyhow::Error`, and `ketchup-program` must report it with the code `memory_limit` like the
worker's own memory limit.

`src/lib.rs` also allows two lints that newer rustc reports in the unchanged upstream
code (`deprecated`, `mismatched_lifetime_syntaxes`), so the build stays quiet.

Changed files, each marked with a "Modified for Kecup" line under its license header
(Apache-2.0 §4(b)): `src/lib.rs`, `src/eval/bc/writer.rs`, `src/eval/compiler/stmt.rs`,
`src/values/layout/heap/heap_type.rs`, `src/values/layout/value.rs`,
`src/values/types/{list,tuple}/{globals,value}.rs`,
`src/values/types/string/{methods,str_type,dot_format,globals,interpolation}.rs`,
`src/values/types/bigint.rs`, `src/values/types/int/pointer_i32.rs` and
`src/stdlib/json.rs`. `LICENSE` is the Apache License 2.0 the
upstream project (https://github.com/facebook/starlark-rust) is published under; the
crates.io package does not carry it.

Why: `ketchup-program` bounds program evaluation (`execution_budget.rs`) through
`before_stmt`. Upstream emits the hook only before statements, so a comprehension
(`[0 for a in L for b in L]`) or a loop whose body is only `pass` iterated without
reaching it and could run without limit.

Remove this copy and the `[patch.crates-io]` entry in the root `Cargo.toml` once an
upstream release counts loop iterations and the workspace can move to it (0.14 turns on
serde_json `arbitrary_precision`, see `crates/ketchup-program/Cargo.toml`).
