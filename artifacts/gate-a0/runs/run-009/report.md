# Gate A0 Report

- Run: `run-009`
- Freeze: `r0-v9`
- Lock SHA-256: `da0dbcd3b3daf845a83f6a708a528c7cdcbf8e0155d1d93bfbb9637c539a7b25`
- **Decision: GO**

## Results

| Contract | Result |
|---|---:|
| Fixed baseline valid | 4/4 |
| Structure-aware FFI fuzz | 10000/10000 |
| Adversarial expected-valid | 10/10 |
| Adversarial non-pass structural diagnosis | 0/0 |
| Expected typed rejections | 6/6 |
| Silent invalid shapes | 0 |
| Guaranteed correct identity | 24/24 |
| Guaranteed history evidence | 24/24 |
| Silent wrong identity | 0 |
| External STEP fixtures | 3/3 |
| Prior-backend references migrated | 3 |
| Unresolved migration references quarantined | 1 |

## Failure consequences

None.

The report was produced by `cargo test -p ketchup-exact --test gate_a0` against the active immutable lock. Geometry fingerprints were used only as corroborating evidence; Guaranteed identity was resolved from producer role, source element lineage, and backend history.
