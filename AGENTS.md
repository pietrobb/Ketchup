# Rules for coding agents working on Ketchup

These rules apply to every agent (Codex/GPT, Claude, or others) and to human contributors.
They override conflicting guidance in older documents, including `docs/*/EXECUTION_CONTRACT.md`
§11, §12 and §16 and the gate/preregistration process.
Plan and rationale: `docs/plan-upratovania-2026-09-24.md`, `docs/analyza-fundamentov-2026-09-24.md`.

## 1. No bespoke shapes or products in the core

- Rust crates contain **only generic geometric operations**: sketch, profile, extrude, revolve,
  sweep, loft, boolean, shell, fillet, chamfer, offset, transform, pattern, generic joint,
  collision/contact/support.
- Before adding anything, ask: *"Would someone modeling something completely different use this?"*
  If not, it does not belong in Rust.
- Domain concepts (board, dowel, groove, shelf, cabinet, drawer, beam, roof, stair, hardware
  catalogs) live **only** in the program language library (`crates/ketchup-program/library/`),
  examples, and test fixtures.
- Forbidden everywhere: named-product types, commands, features or face roles (bottle, teapot,
  balloon letters, capsule, nightstand, …); evaluators that support one shape; hand-written meshes
  for specific shape combinations; features with a fixed point count; modules written for one test.
- Format exporters (STEP, BTLx, HOMAG, DXF, …) are allowed. A format is not a product.
- If a generic operation is missing, **extend the generic operation**. Do not add a special case
  next to it.

## 2. Errors must tell the caller what to fix

- Every rejection carries `code`, `path` (the field or program line), `message` in plain language,
  and `hint` (a concrete fix). Geometric failures also name the parts and a location in mm.
- Never discard an error's content: no `map_err(|_| "...")`, no rewriting of messages in the SDK
  or skills, no collapsing many causes into one code.
- Rules the caller must satisfy (for example derived hole depths) belong in the schema/library
  documentation, not only in the rejection.

## 3. Validation reports; it does not block editing

- Validators return a list of issues. Edits are applied even when issues exist.
- Blocking is allowed only for:
  - production export,
  - violations of document integrity (unknown IDs, cycles, non-finite numbers).
- Validation runs locally over the changed parts and their neighbours, not over the whole model
  on every command.

## 4. Keep only the checks that protect data

**Keep:**
- one mutation path (`apply_batch`), atomic batches, one undo step per batch;
- a revision number on asynchronous OCCT results, with stale results dropped;
- worker process isolation and timeouts;
- no silent mesh-for-exact substitution;
- a checksum on saved files.

**Do not add** new digests, fingerprints, stamps, epochs, receipts, signatures, preregistrations,
freeze IDs, frozen-hash locks or evidence documents. The only exceptions are a concrete,
reproduced failure that they catch, or explicit approval by the owner.

## 5. Tests assert outcomes

- Test geometry and product outputs: volumes, bounds, contacts, collisions, bill of materials,
  drilling plans, exported files. Prefer golden tests of whole programs (cabinet, nightstand,
  timber beam).
- Do not write tests whose main assertions are digests, revision numbers or rejection codes of
  internal plumbing.
- Do not generate combinatorial per-shape test matrices.
- When a feature is deleted, delete its tests. Do not port them.

## 6. Delete more than you add

- When a new path replaces an old one, remove the old one in the same change. No parallel
  "legacy" paths.
- Do not grow `crates/ketchup-app/src/lib.rs` or `crates/ketchup-core/src/document.rs`. Put new
  code in new modules.
- New code: functions under ~150 lines, files under ~3000 lines.
- Report the net line delta in every PR description.

## 7. Communication

- Commit messages and PR descriptions are short, plain prose: what changed and why.
- No walls of hashes, nonces, test counts or "evidence" paragraphs.
- Code, identifiers, schemas and error codes are in English (ADR 0001).

## 8. Definition of done

1. `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo fmt --all -- --check` pass.
2. The named-product check passes, once `scripts/check_no_named_products.py` exists. It is a
   ratchet: the baseline may only shrink.
3. Manual smoke test:
   - open the examples;
   - draw a rectangle and Push/Pull it;
   - save and reopen;
   - once the program language exists, run the cabinet program and export the bill of materials.
