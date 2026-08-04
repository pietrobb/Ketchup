# R0 v13 Preregistration Report

**Decision: GO**

- Freeze: `r0-v13`
- Lock SHA-256: `b1cf0c769cb46d0c678c1bc579e241356cc85663582a0df72093e2e54086cb01`
- Measurement state at freeze: `not_started`
- Frozen Gate C build-input tree SHA-256: `de8592b10b5ed88d2ae7cf8394c127d3d7ca1ea8b22830911cc28a8fbdca84bb`

## Authorized repair

Operator usability review before any HP-IGPU-01 observation found incorrect cuboid occlusion and selected-face presentation, disagreement between the camera and exact picking, undiscoverable Push/Pull interaction, and an artificial orbit-pitch limit. R0 v13 authorizes only the repaired product viewport, localized English guidance, unrestricted orbit, and the provenance metadata required to bind those changes to the formal Gate C runner.

All thresholds, corpora, expected outcomes, hardware profiles, oracles, consequences, dependencies, toolchain evidence, and OCCT inputs remain unchanged. Historical r0-v9 through r0-v12 observations remain immutable and cannot certify the repaired executable. New HP-DEV-01 reference series and any HP-IGPU-01 formal series must use this exact lock and build-input-tree hash; no r0-v13 Gate C measurement existed when this lock was frozen.
