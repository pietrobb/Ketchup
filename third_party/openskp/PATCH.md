# openskp 1.5.0 (MIT), vendored for Ketchup

Copy of the Rust package of OpenSKP (`packages/rust` of
https://github.com/iamahsanmehmood/openskp, commit `c98561147ea502c6d4d53a257fa4581d0c6cde1d`,
9 November 2026; MIT, see `LICENSE`). Only `src/`, `Cargo.toml`, `LICENSE`, `README.md` and
`.gitattributes` are copied; the upstream `tests/` directory and its fixtures are left out.

Why vendored: the package is not published on crates.io. The crate named `openskp` there is a
different project under GPL-3.0 (github.com/hew3d/openskp), which Ketchup must not use; a path
dependency never resolves to it.

Ketchup reads `.skp` files through `crates/ketchup-skp`, which calls only the reader
(`build_instanced_scene`, `parse_skp`) and the instanced GLB writer of this crate, in the program
worker: the reader trusts sizes a file declares and recurses over its nesting, so a hostile file
must cost a worker process, not the editor.

## Ketchup changes

- `src/face_groups.rs`, `TextureRegistry::index_for`: upstream tells images apart by length
  and their first 16 bytes, which every PNG of one pixel size often shares, so two different
  pictures became one. Images are now told apart by their bytes, compared once per image
  buffer. Test: `two_different_pictures_of_the_same_file_size_stay_two` in `ketchup-skp`.
- `src/instanced_scene.rs`, `src/instanced_scene_build.rs`: `InstancedNode::hidden` carries a
  placement's own hidden flag (SketchUp's Hide), which upstream read into `RawInstance` but
  dropped from the scene. Test: `a_hidden_placement_comes_hidden_by_node` in `ketchup-skp`.
- `src/face_groups.rs`, `build_local_face_groups`: upstream writes a face whose two sides are
  painted differently twice, the back as a reversed copy. A solid painted on its outside only
  (SketchUp's paint bucket) then came with a second, inside-out shell, which Ketchup's GLB
  import took as sheets (review 2026-11-10 SKP-400). Every face is now written once, on its
  front, drawn from both sides, in its front's material, or its back's when only the back is
  painted; `ketchup-skp` counts a back painted unlike its front as
  `skp_back_materials_dropped`. Tests: `a_box_painted_on_its_outside_only_comes_as_one_surface`
  and `a_face_painted_on_its_back_only_keeps_that_colour` in `ketchup-skp`.

To update: copy the same files of a newer upstream commit over this directory, record the commit
here, keep or drop each change above (drop one only when upstream fixed it), and run the tests
of `ketchup-skp`.
