# Historical Gate A0 audit addendum

Date: 2026-08-03
Disposition: evidence-scope correction; historical artifacts remain immutable.

## Scope retained

The `run-011` GO remains valid only for the frozen r0-v11 operation envelope, fixed/adversarial/fuzz corpus, typed diagnostics, STEP fixtures, and the synthetic rectangular semantic-role resolver that the run actually exercised. It is not certification of the current working tree or of a general topological naming system.

## Independent audit findings

1. **Adjacency evidence was incomplete.** The native facade recorded face geometry and an aggregate edge count, but did not record stable edge identities, each face's complete boundary, each edge's adjacent faces, or reciprocal face/edge consistency. The historical 24/24 Guaranteed result therefore proves semantic-role selection and selected history rows, not the complete adjacency evidence required by the frozen Guaranteed contract.
2. **Backend migration was synthetic.** `run-011` changed only the stored backend-fingerprint string and resolved the reference against an output produced by the same loaded backend build. It did not transfer references from one real backend binary build to another.
3. **The current tree is not covered.** The r0-v13 validation currently fails closed when current locked inputs differ. That is a provenance failure before geometry observation and must not be reported as geometry evidence.

## Required replacement claim

No new A0 claim may rely on the historical migration or adjacency rows. Strengthened A0 v1 must be preregistered before observation, freeze its exact implementation and two real backend install trees, prove complete reciprocal adjacency for every Guaranteed outcome, transfer serialized references from a producer linked to the prior build into a consumer linked to the current build, and classify any failure as either `hash_or_provenance_only` or `substantive_topology_or_reference`.

A hash/provenance-only failure authorizes no geometry conclusion and requires a new pre-observation freeze. A substantive failure blocks M1/M2/M3 until an explicit planar fallback or redesign disposition is recorded.
