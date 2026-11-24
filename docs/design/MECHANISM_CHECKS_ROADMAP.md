# Roadmap: "Will it work?" checks for machines and mechanisms

**Goal.** A person who builds a machine for the first time models it in Ketchup and, before cutting
any steel, sees whether it will work under load. Under load it may settle somewhere unexpected:
- a frame sags until a moving part touches something;
- a chain goes slack;
- a drive wheel loses grip;
- a shaft binds in misaligned bearings;
- a long screw whips at speed.

Ketchup names the problem in plain words, says why it happens, and proposes a fix. A typical fix is
"add a spring of about 25 N/mm with 200 N preload".

This is not a dynamics simulator such as Adams, Simscape or RecurDyn. Most first-build failures are
answered by one question: **where does the assembly settle under load, and does it still work in
that state?** That is a quasi-static equilibrium of a flexible mechanism with one-sided contacts.
It is fast, it runs automatically after every change, and each result can be explained in a
sentence.

Every report says which checks it made and which it did not. A check Ketchup does not make is
listed as "not checked", never shown green.

This plan is ordered. Each work package (WP) states:
- where the code goes;
- what "done" means;
- its rough size: S is up to a few days, M about a week, L several weeks.

The rules of `AGENTS.md` apply throughout:
- **The core gets only generic elements:** a spring, a flexible drive with slack, a one-sided
  contact with friction, a resisting torque, a bearing that tolerates some misalignment.
- **Catalogues are data in the library, with a source:** chain sizes such as 08B, tyres, bearing
  units such as UCP, springs, bulk densities of sand or grain, and brush drag.
- **Validators against a standard are allowed:** a simplified EC3 member check, a bearing L10 life
  or a chain working load. Each one names its standard and its limits.

## 1. Acceptance cases

Three real machines drive the plan. Each WP below says which case it moves forward. A case is done
when its scripted test in `crates/ketchup-application/tests/` passes and the "Will it work?" report
for it reads correctly to a person who has not seen the code.

### 1.1 Towed sand spreader (the owner's machine, drawn in SketchUp in 2020)

**The machine:**
- A towed frame on wheels carries a hopper with about one tonne of sand.
- A ground wheel in the middle turns a brush through a chain drive.
- The brush meters sand out through a gate under the hopper.

**What happened in the workshop:**
- In the first trial there were no springs and no chain tensioner. Under the full hopper the
  machine could not move at all: something jammed.
- Springs, sized by hand calculation, and a tensioner fixed it.

**What Ketchup must show:**
- **Without the springs and the tensioner:** with the hopper going from empty to full, the report
  turns red for at least one of these, with the reason in numbers:
  - the brush-to-gate clearance closes;
  - the chain slack exceeds what the sprocket tolerates;
  - the drive wheel's transmittable torque drops below the brush's required torque.

  Which one it is, is a hypothesis to compare with the owner's memory. It must not be tuned to
  match.
- **With the springs and the tensioner:** every check is green from empty to full, on a 10 %
  slope and on a bump with a dynamic factor of 2.
- **Spring design:** the spring-design helper (WP G1) proposes a spring stiffness and preload. The
  proposal must be within the range the owner chose by hand.

**Input:**
- The owner's `.skp` model is imported through `ketchup-skp`.
- Springs, the chain, the brush and the tensioner are added as functional elements (phase A).
  The model does not have them drawn.

### 1.2 CNC router for wood

**What Ketchup must show:**
1. **Critical speed of a long ball screw**, a SFU1605 screw 2 m long with a root diameter of about
   12.9 mm:
   - The catalogue formula `n_cr = f · d_r / L² · 10^7` gives about 310 rpm with both ends supported
     (f = 9.7) and about 710 rpm with both ends fixed (f = 21.9). These factors already include the
     catalogue's 0.8 safety factor.
   - The modal solve of the screw as a beam gives about 393 rpm for the supported case.
   - Ketchup reports both values and says which one it applies. Each must be within 2 % of the
     closed form.
2. **Stiffness at the tool tip:** the deflection at the tool tip under a cutting force, in X, Y
   and Z, through the gantry, the carriages and the frame.
3. **Natural frequencies of the gantry** compared with the spindle speed and the tooth-passing
   frequency.
4. **A belt drive instead of a screw** for the long axis: no whip limit. The report must say why
   the belt is not limited by critical speed. The owner's second single-axis machine used a rubber
   belt and worked perfectly.

### 1.3 Simple two-wheel towed cart

A hand-checkable reference for the solver:
- **Statics:** one axle, a hitch, and a load of known mass at a known centre of gravity. The axle
  load and the hitch load come from one moment balance. Ketchup must match it to 0.1 %.
- **Lift-off:** a centre of gravity behind the axle must give a negative hitch load. Ketchup must
  report it as "the drawbar lifts".
- **Tip-over:** on a side slope, tip-over must be reported at the angle where the resultant leaves
  the wheel track.

## 2. State at `86b606ba` (0.3.3), checked against the code

The pieces below already exist; the plan builds on them and does not duplicate them.

### Kinematic joints

`ketchup-model/src/assembly_joint.rs`, `AssemblyJointKind`, has these kinds:
- `Fixed`;
- `Revolute`, with an axis, optional limits and a position;
- `Prismatic`;
- `Helical`, with a lead.

### Mates

`ketchup-model/src/assembly.rs`, `AssemblyMateKind`, has these kinds:
- coincident planar;
- concentric axial;
- distance;
- angle.

### Transmissions

`ketchup-model/src/mechanical_coupling.rs`, `AssemblyTransmissionKind`, couples two joints with
one of:
- a gear pair;
- a belt, with pitch diameters;
- a chain, with tooth counts;
- a rack and pinion;
- a lead screw.

These are ideal ratios. A transmission does not know its centre distance, so it can never go slack
or overload.

### Motion and collision

`ketchup-application/src/collision_motion*.rs` sweeps a joint motion and finds collisions.
`contact_joints.rs` checks declared contacts against the exact solids.

### Mass properties

`ketchup-application/src/mass_properties.rs` computes:
- the mass of each part and of the assembly;
- the centre of mass;
- the inertia.

They come from the exact solids and a density with a source. A part without a density is listed
and left out, and the assembly is then marked partial.

### FEA

`ketchup-analysis` has:
- truss, tetrahedron and `Frame2` beam elements (`fea.rs`, `fea_frame.rs`);
- rigid and spring couplings (`FeaCouplingKind::Rigid` and `Spring`);
- a modal solve (`fea_modal.rs`);
- a convergence study.

`ketchup-scheduler/src/fea_assembly.rs` builds an assembly model, with two limitations:
- It does not expose `Frame2` to the user.
- It turns face joints into a rigid plate. Review 2026-11-19 (og), finding FEA-800, measured this
  as about 4.4 times too stiff. The equilibrium below inherits that error until it is fixed.

### Timber checks

`ketchup-program` checks timber after EC5 (`member_check.rs`, `loads.rs`). Steel and aluminium
members have no check.

### SketchUp import

`ketchup-skp` imports `.skp` files. That is the way in for test case 1.1.

### What is missing

- **Functional elements:** springs, slack in a drive, one-sided ground contact with friction, a
  resisting torque, bearing misalignment.
- **Load cases for machines:** contents, operation, dynamic factors.
- **A solver** that settles a mechanism with flexible frames and one-sided contacts.
- **Checks on the settled state**, and design helpers.
- **A report** that reads like a person talking.

## 3. Phase A: functional elements (L)

**A1. Spring element (S).**
- **Model:** add `AssemblySpring` to `ketchup-model`, between two attachment points on two
  occurrences. Its data:
  - stiffness in N/mm;
  - free length;
  - preload, or installed length;
  - allowed compression and extension;
  - optional catalogue reference.
- **Validation:** the same style as `AssemblyMateKind::is_valid`.
- **Done:** the spring persists and round-trips; it appears in the joints panel and over MCP
  `joints`.

**A2. Flexible drive with centre distance (M).**
- **Model:** extend `AssemblyTransmissionKind::Belt` and `Chain` with:
  - the pitch, or the belt length;
  - the installed centre distance;
  - the allowed slack and the allowed tension, from the catalogue.
- **Tensioner:** an `AssemblyTensioner` element, a prismatic or a pivoting idler with its own
  travel and an optional spring from A1.
- **Done:** the drive computes its slack from the actual centre distance of the two shafts in any
  pose.

**A3. One-sided contact with friction (M).**
- **Model:** add `AssemblyGroundContact`:
  - a wheel or a foot on a plane (ground or slope);
  - its radius, its radial stiffness and a friction coefficient.
- **Behaviour:**
  - The contact carries only compression, so it can lift off.
  - It transmits a tangential force up to μ times the normal force, so it can slip.
- **Part-to-part contact:** the same element works between two parts, for a brush on a gate or a
  roller on a rail.
- **Done:** the contact persists, shows in the view, and the solver in phase C accepts it.

**A4. Resisting torque and resisting force (S).**
- **Model:** on a revolute or prismatic joint, a resisting load. It is either:
  - a constant;
  - a value per unit speed;
  - a value from library data, for example brush drag in sand per metre of brush.
- **Done:** a brush shaft declares "needs 35 N·m to turn".

**A5. Bearing with allowed misalignment (S).**
- **Model:** a revolute joint gets an optional bearing record:
  - the allowed tilt, for example 2° for a self-aligning unit or 0.05° for a rigid ball bearing;
  - the load ratings C and C0, from the library.
- **Done:** the bearing record round-trips and the library has a few common units with a source.

**A6. Library catalogues (M).**
- **Data:** in `crates/ketchup-model/library/` or the program library, each value with a source:
  - roller chains 06B, 08B, 10B and 12B, with pitch, breaking load and working load;
  - V-belts and timing belts;
  - a few tyre sizes, with load index and stiffness;
  - UCP/UCF bearing units;
  - compression springs;
  - bulk densities of sand, gravel, grain and salt.
- **Done:** the data has a schema test, and every entry carries `source`.

## 4. Phase B: loads as people think of them (M)

**B1. Self-weight from `mass_properties` (S).**
- **Behaviour:** gravity on every part, at its centre of mass.
- **Missing density:** a part without a density stops the check with a clear message. The check
  does not run partially.
- **Done:** the cart of case 1.3 gets its weight with no input.

**B2. Contents (M).**
- **Input:** a closed or open container part plus:
  - a fill level, either to the brim or to a height;
  - a bulk material from the library.
- **What Ketchup computes:**
  - the mass and its centre;
  - a simplified wall pressure, Janssen for a silo or hydrostatic times a factor for a hopper.
    The wall pressure is labelled "simplified".
- **Done:** "hopper full of sand" gives about one tonne for the owner's hopper volume, and the
  report shows the volume and density used.

**B3. Operation loads (S):**
- travel speed;
- towing at a hitch;
- slope, along and across;
- curve radius, as a lateral acceleration;
- a dynamic factor for bumps, entered or taken from the library with a source.

**B4. Load cases (S).**
- **Input:** named combinations of B1–B3, for example "empty", "full", "full in a curve" and "full
  on a bump".
- **Done:** all of them are checked in one run.

## 5. Phase C: equilibrium of a flexible mechanism (L)

**C1. Solver core (L).**
- **Where:** a new module `ketchup-analysis/src/mechanism.rs`.
- **Unknowns:** the rigid-body poses of the parts, the joint positions and the frame deflections.
  The frame deflections come from `Frame2` elements reused from `fea_frame.rs`.
- **Method:** Newton iterations on a nonlinear system with an active set, for:
  - contacts that lift off;
  - contacts that slip;
  - drives that go slack (a chain carries no compression).
- **Load path:** load increments from 0 to 100 %, so that the report can show at which load a
  problem starts.
- **Done:** case 1.3 matches its hand calculation to 0.1 %, and a spring-supported beam matches
  its closed form.

**C2. Expose frames to the user (M).**
- **Input:** a steel or aluminium member drawn as a part with a profile becomes a `Frame2` line.
  The section properties come from the profile, and the node layout follows the existing beam
  layout.
- **Depends on:** the fix of FEA-800, so that face joints become springs or partial couplings
  rather than a rigid plate.
- **Done:** the owner's frame from the `.skp` file becomes a beam model without redrawing.

**C3. Build the model from the document (M).**
- **Where:** `ketchup-application/src/mechanism_workflow.rs`, which collects:
  - occurrences, joints and mates;
  - A1–A5 elements;
  - B1–B4 loads;
  - masses.
- **Rejections:** each unsupported item is rejected with a reason, never silently dropped.
- **Done:** the result carries the digest of its inputs, like the FEA results do.

**C4. Honest convergence (S).**
- **Behaviour:**
  - The solver reports its iterations, its residual and every contact that kept switching.
  - A model that does not converge is reported as "unknown", never as passed.
  - A mechanism that has a free motion under load is reported as "it moves here". An example is a
    pivot with nothing holding it. This is itself a finding.

## 6. Phase D: checks on the settled state (L)

These checks answer the workshop's question of why it could not move. Each check produces:
- a status: OK, warning, fail or not checked;
- the numbers;
- one sentence on what is wrong, one on why, and one on the fix.

**D1. Clearances after deflection (M).**
- **Method:** reuse the exact collision code (`collision*.rs`) on the settled poses.
- **Result:** the minimum distance between parts that move relative to each other, and every
  declared clearance.
- **Example:** "the brush touches the gate flap at 70 % fill; the frame sags 11 mm at the hopper
  middle".

**D2. Drives (S).**
- **Result:** the change in centre distance, the resulting slack or tension, and the travel the
  tensioner needs and has.
- **Example:** "the centre distance shrinks by 9 mm, the chain slack is 3 times the allowed sag, it
  can jump the sprocket".

**D3. Wheels and contacts (S):**
- the load on each contact against its rating;
- lift-off;
- the drawbar load and its sign.

**D4. Shaft alignment (S).**
- **Result:** the tilt between the two bearings of one shaft, after deflection, against the
  allowed tilt of each bearing.

**D5. Torque flow: does it turn? (M)**
- **Method:** walk the transmission graph from the driving contact to every resisting load
  (A4):
  - the required torque on each shaft, through the ratios and an efficiency;
  - the torque the driving contact can transmit, μ · N · r, with N from the settled state.
- **Result:** one of these:
  - **turns**;
  - **slips**, with the shortfall in N·m;
  - **blocked**, when D1 found a contact on a moving part.
- **Towing:** the force needed at the hitch.

**D6. Stability (S):**
- tip-over along and across a slope;
- drawbar lift-off;
- reported as the angle, or the fill, at which it happens.

## 7. Phase G: design helpers (M)

**G1. Size a spring (S).**
- **Input:** "the drive wheel must press with at least X N from the empty case to the full case".
- **Method:** Ketchup sweeps the stiffness and the preload, then proposes the softest catalogue
  spring that holds X in every case and stays within its allowed compression.

**G2. Size a tensioner (S).**
- **Method:** the travel needed is the largest change in centre distance over all load cases, plus
  an elongation reserve. The reserve comes from the catalogue: by default the wear elongation of a
  roller chain.

**G3. Sweeps (S).**
- **Input:** any load parameter, for example a fill from 0 to 100 % or a slope from 0 to 10 %.
- **Result:** a chart of each check's margin against that parameter, with the first failing point
  marked.

## 8. Phase E: strength and life (L)

**E1. Steel and aluminium members (L).**
- **Method:** simplified member checks after EC3 (EN 1993-1-1) and EC9 (EN 1999-1-1):
  - tension, compression and bending;
  - flexural buckling.

  These are validators with a stated scope, as EC5 is today.
- **Not checked:** lateral-torsional buckling, unless implemented, plus welds and bolts. All of
  them are listed as "not checked".

**E2. Bearing life (S).**
- **Method:** L10 from C and the equivalent load, at the speed from F3.

**E3. Chain and spring stresses (S):**
- the chain tension against its working load;
- the spring stress against its catalogue limit.

## 9. Phase F: speeds and resonance (M)

**F1. Critical speed of shafts and screws (S).**
- **Methods:** the catalogue formula, plus the modal solve of the shaft as `Frame2` beams with
  the actual bearing layout. The report shows both and names which one governs.
- **Done:** case 1.2, item 1.

**F2. Frame natural frequencies (M).**
- **Method:** `fea_modal.rs` on the frame of C2.
- **Comparison:** against every speed from F3 and its first harmonics.
- **Result:** a warning within ±15 %.

**F3. Speed chain (S).**
- **Method:** from the travel speed through the ground contact and every ratio to each shaft's
  rpm, the brush surface speed and the screw rpm.

## 10. Phase H: show it (runs alongside every phase)

**H1. "Will it work?" panel (M).**
- **Controls:** a load-case switcher.
- **Content:** one row per check, with a status colour.
- **Each row:** what, why, the fix, and what was not checked.

All strings are localized, in English, Slovak and Czech, as the rest of the app.

**H2. Deformed view (M):**
- the settled pose, exaggerated by a stated factor, over the undeformed model;
- a "fill from 0 to 1 t" scrub that animates C1's load path, so you can see the chain slowly go
  slack.

**H3. MCP tools (S).**
- **Tools:** `mechanism_check`, `mechanism_sweep` and `mechanism_design_spring`, which return the
  same structured result as the panel.
- **For an AI:** it can run the checks, explain them and propose a fix. It changes the model only
  through the normal command path.

**H4. Report export (S).**
- **Format:** HTML or PDF, like the calculation report.
- **Content:** the input digest and the "not checked" section.
- **Lesson from review 2026-11-19 (og), PG-402:** a report without an issues section hides the
  failures.

## 11. Order and dependencies

1. **A1–A4 with B1–B4:** enough to describe the sand spreader and the cart.
2. **C1, C3, C4 on rigid parts with springs and contacts:** case 1.3 passes.
3. **The FEA-800 fix, then C2:** flexible frames, the spreader's frame sag.
4. **D1–D6:** the spreader's red report without springs and without the tensioner.
5. **G1–G3:** the green report with them, and the spring proposal.
6. **F1–F3, then E1–E3:** case 1.2.
7. **H1–H4:** alongside every step. Each check is visible as soon as it exists.

## 12. Out of scope

These stay out of scope until the cases above pass:
- time-domain dynamics: impacts, vibration amplitudes, start-up transients;
- granular flow of the sand itself (DEM);
- fatigue of welds;
- tyre models beyond a linear stiffness and Coulomb friction;
- thermal effects.

Each of these, when it matters, is listed in the report as "not checked".
