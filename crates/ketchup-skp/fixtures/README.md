SketchUp files from the test fixtures of OpenSKP (https://github.com/iamahsanmehmood/openskp,
`packages/rust/tests/fixtures`, commit `c98561147ea502c6d4d53a257fa4581d0c6cde1d`; MIT,
Copyright (c) 2024-2026 Ahsan Mehmood, see `third_party/openskp/LICENSE`).

- `SU_File.skp`: saved by SketchUp 2025 (the container format of 2021 and newer); one closed solid.
- `nesting_2013.skp`, `nesting_2017.skp`, `nesting_2020.skp`: the same nested groups saved by
  SketchUp 2013, 2017 and 2020 (the classic archive format).

Written by Ketchup's own tests:

- `tagged_boxes.skp`: `tagged_boxes(true)` of `src/tests.rs` (OpenSKP's writer, classic format):
  a box component placed twice on the tag "Walls", hidden in the file, one face on the tag "Roof".
