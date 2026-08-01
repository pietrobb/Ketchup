# A0 Diagnosis after run-002

## Preserved evidence

- `run-001` is an immutable NO-GO: 16/24 Guaranteed identities passed and 8 east-side identities were wrong.
- Root cause: OCCT reports the geometrically planar lateral face of a linear prism as `SurfaceOfExtrusion`. The inspector classified it as `other`, and the semantic fallback selected a cap. The backend/reference model was reopened and fixed with geometric plane detection.
- `run-002` is an immutable NO-GO: 0/24 Guaranteed identities passed while all 24 lineage and history records remained complete.
- Root cause: `BRepBuilderAPI_FindPlane` supplies a valid plane whose normal orientation is not a stable outward-identity oracle. The A0 test had added a non-frozen normal-sign requirement beyond the frozen Guaranteed contract.

## Corrected oracle before any later run

Guaranteed identity remains strict and requires all of the following:

1. exactly one match by producer semantic role and stable source element;
2. backend history identifies an existing output face;
3. the output is geometrically planar;
4. the semantic extreme is correct for the frozen axis-aligned producer: top at `z=height`, bottom at `z=0`, east side at `x=width`;
5. geometry fingerprint is corroborating evidence only;
6. Ambiguous, Lost, or unresolved backend migration cannot pass.

Removing the arbitrary normal-sign check does not change a threshold, corpus, semantic role, expected identity, or failure consequence.

## Resolution

The focused three-role regression, Clippy, and active-lock validation passed before `run-003`. Immutable `run-003` then passed all frozen A0 thresholds: 10,000/10,000 structure-aware FFI cases, 24/24 Guaranteed identities and history records, zero silent invalid shapes or wrong identities, 10/10 expected-valid adversarial cases, 6/6 typed rejections, 3/3 STEP fixtures, three resolved prior-backend references, and one quarantined unresolved migration. The active A0 report is GO; `run-001` and `run-002` remain preserved as NO-GO evidence.
