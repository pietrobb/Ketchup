//! Review 2026-10-09 §5.1 (9): every import reader takes mutated files and
//! either reads them or refuses them, never panics and never runs long. The
//! mutations are deterministic (fixed seeds), so a failure names the case that
//! reproduces it. The per-input time limit catches blowups such as DXF-1 (one
//! block inserted thousands of times), the panic check catches DXF-2 (an
//! `unreachable!` behind a rotation a hair below zero).

use ketchup_model::import::{
    DxfImportOptions, ImportLengthUnit, ImportUnitAuthority, ImportUnitDecision, inspect_dxf,
    inspect_glb, inspect_sketchup_scene, parse_stl,
};
use std::sync::mpsc;
use std::time::Duration;

/// Generous for a debug build on a loaded machine; the seeds read in
/// milliseconds, so only a pathological input comes near it.
const PER_INPUT: Duration = Duration::from_secs(5);

/// xorshift64*: small, fast and the same on every machine.
struct Mutator(u64);

impl Mutator {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % bound.max(1) as u64).unwrap()
    }

    /// Values that sit on the edges readers get wrong: signed zero, a hair
    /// below zero, overflow, not-a-number, huge counts and structure tokens.
    const TOKENS: &'static [&'static [u8]] = &[
        b"-0",
        b"-2.8e-14",
        b"1e308",
        b"-1e308",
        b"NaN",
        b"inf",
        b"4294967295",
        b"18446744073709551616",
        b"-1",
        b"0",
        b"360",
        b"\n0\n",
        b"\n",
        b"\"",
        b"{",
        b"}",
        b"[",
        b"]",
        b",",
        b"null",
        b"1e-320",
    ];

    fn mutate(&mut self, seed: &[u8]) -> Vec<u8> {
        let mut bytes = seed.to_vec();
        for _ in 0..=self.below(3) {
            if bytes.is_empty() {
                bytes.push(b'0');
            }
            let at = self.below(bytes.len());
            match self.below(8) {
                0 => bytes[at] ^= 1 << self.below(8),
                1 => bytes.truncate(at),
                2 => {
                    let end = (at + 1 + self.below(256)).min(bytes.len());
                    let copy = bytes[at..end].to_vec();
                    let to = self.below(bytes.len());
                    bytes.splice(to..to, copy);
                }
                3 => {
                    let end = (at + 1 + self.below(64)).min(bytes.len());
                    bytes.drain(at..end);
                }
                4 => {
                    let token = Self::TOKENS[self.below(Self::TOKENS.len())];
                    bytes.splice(at..at, token.iter().copied());
                }
                5 => {
                    // Replace the number the position falls into.
                    let is_number =
                        |byte: u8| byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'e');
                    let mut start = at;
                    while start > 0 && is_number(bytes[start - 1]) {
                        start -= 1;
                    }
                    let mut end = at;
                    while end < bytes.len() && is_number(bytes[end]) {
                        end += 1;
                    }
                    let token = Self::TOKENS[self.below(Self::TOKENS.len())];
                    bytes.splice(start..end, token.iter().copied());
                }
                6 => {
                    // Repeat one record thousands of times, as a file that
                    // inserts one block over and over (DXF-1) does.
                    let (start, end) = record_around(&bytes, at)
                        .unwrap_or((at, (at + 1 + self.below(128)).min(bytes.len())));
                    let record = bytes[start..end].to_vec();
                    let repeated: Vec<u8> = (1..=2 + self.below(4000))
                        .flat_map(|copy| moved_record(&record, copy))
                        .collect();
                    bytes.splice(end..end, repeated);
                }
                _ => bytes[at] = u8::try_from(self.below(256)).unwrap(),
            }
        }
        bytes
    }
}

/// The DXF record (from one `0` group code to the next) around `at`, so that a
/// repeated record stays well formed.
/// Lines are read in code/value pairs, since a value line may itself be `0`.
fn record_around(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let mut line_starts = vec![0];
    line_starts.extend(
        bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n')
            .map(|(index, _)| index + 1),
    );
    let record_starts: Vec<usize> = line_starts
        .chunks(2)
        .filter(|pair| pair.len() == 2 && bytes[pair[0]..pair[1]].trim_ascii() == b"0")
        .map(|pair| pair[0])
        .collect();
    let next = record_starts.partition_point(|&start| start <= at);
    let start = *record_starts.get(next.checked_sub(1)?)?;
    let end = *record_starts.get(next)?;
    (end - start <= 512).then_some((start, end))
}

/// The record with its first x coordinate (group code 10) moved by `copy`
/// steps, so that its copies stand apart and are not refused as duplicates;
/// 4 000 steps stay within the coordinate limit.
fn moved_record(record: &[u8], copy: usize) -> Vec<u8> {
    const X: &[u8] = b"\n10\n";
    let Some(code) = record.windows(X.len()).position(|window| window == X) else {
        return record.to_vec();
    };
    let value = code + X.len();
    let Some(length) = record[value..].iter().position(|&byte| byte == b'\n') else {
        return record.to_vec();
    };
    let mut moved = record[..value].to_vec();
    moved.extend_from_slice(format!("{}", copy * 200).as_bytes());
    moved.extend_from_slice(&record[value + length..]);
    moved
}

/// Runs every mutated input on its own thread and fails on a panic or on an
/// input that takes longer than `PER_INPUT`.
fn fuzz(format: &str, seeds: &[Vec<u8>], cases: usize, read: fn(&[u8])) {
    let mut mutator = Mutator(0x9E37_79B9_7F4A_7C15 ^ format.len() as u64);
    for case in 0..cases {
        let seed = &seeds[case % seeds.len()];
        let input = if case < seeds.len() {
            seed.clone()
        } else {
            mutator.mutate(seed)
        };
        let length = input.len();
        let (done, finished) = mpsc::channel();
        let reader = std::thread::Builder::new()
            .name(format!("{format}-fuzz-{case}"))
            .spawn(move || {
                read(&input);
                let _ = done.send(());
            })
            .unwrap();
        match finished.recv_timeout(PER_INPUT) {
            Ok(()) => reader.join().unwrap(),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let panic = reader
                    .join()
                    .expect_err("the reader stopped without finishing");
                let message = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
                    .unwrap_or_default();
                panic!("{format} case {case} ({length} bytes) panicked: {message}");
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                panic!("{format} case {case} ({length} bytes) ran longer than {PER_INPUT:?}");
            }
        }
    }
}

fn dxf_seeds() -> Vec<Vec<u8>> {
    let header = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n";
    let plain = format!(
        "{header}0\nSECTION\n2\nENTITIES\n{}0\nENDSEC\n0\nEOF\n",
        concat!(
            "0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n",
            "0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n",
            "0\nLWPOLYLINE\n8\ncut\n90\n4\n70\n1\n",
            "10\n0\n20\n0\n42\n0\n10\n10\n20\n0\n42\n1\n10\n10\n20\n10\n42\n0\n10\n0\n20\n10\n42\n0\n",
            "0\nCIRCLE\n8\nholes\n10\n5\n20\n5\n40\n1.5\n",
        )
    );
    let blocks = format!(
        "{header}0\nSECTION\n2\nBLOCKS\n{}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{}0\nENDSEC\n0\nEOF\n",
        concat!(
            "0\nBLOCK\n8\n0\n2\nrotated\n70\n0\n10\n1\n20\n2\n30\n0\n",
            "0\nLINE\n8\n0\n10\n1\n20\n2\n11\n3\n21\n2\n",
            "0\nCIRCLE\n8\ndetail\n10\n2\n20\n3\n40\n0.5\n",
            "0\nENDBLK\n8\n0\n",
        ),
        concat!(
            "0\nINSERT\n8\ninstances\n2\nrotated\n10\n10\n20\n20\n41\n2\n42\n2\n50\n-2.8e-14\n",
            "0\nINSERT\n8\ninstances\n2\nrotated\n10\n40\n20\n20\n50\n90\n70\n3\n71\n2\n44\n10\n45\n10\n",
        )
    );
    // One block of many circles inserted many times, within the limits; a
    // repeated INSERT record pushes it past them (DXF-1).
    let circles: String = (0..20)
        .map(|index| format!("0\nCIRCLE\n8\n0\n10\n{}\n20\n0\n40\n1\n", index * 3))
        .collect();
    let inserts: String = (0..8)
        .map(|index| format!("0\nINSERT\n8\n0\n2\nfull\n10\n0\n20\n{}\n", index * 10))
        .collect();
    let repeated = format!(
        "{header}0\nSECTION\n2\nBLOCKS\n0\nBLOCK\n8\n0\n2\nfull\n70\n0\n10\n0\n20\n0\n30\n0\n{circles}0\nENDBLK\n8\n0\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{inserts}0\nENDSEC\n0\nEOF\n"
    );
    vec![
        plain.into_bytes(),
        blocks.into_bytes(),
        repeated.into_bytes(),
    ]
}

fn stl_seeds() -> Vec<Vec<u8>> {
    let facets: [[[f32; 3]; 3]; 4] = [
        [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    ];
    let mut binary = vec![0_u8; 80];
    binary.extend_from_slice(&4_u32.to_le_bytes());
    let mut ascii = String::from("solid fuzz\n");
    for facet in facets {
        binary.extend_from_slice(&[0_u8; 12]);
        ascii.push_str("facet normal 0 0 0\nouter loop\n");
        for vertex in facet {
            for value in vertex {
                binary.extend_from_slice(&value.to_le_bytes());
            }
            ascii.push_str(&format!(
                "vertex {} {} {}\n",
                vertex[0], vertex[1], vertex[2]
            ));
        }
        binary.extend_from_slice(&[0_u8; 2]);
        ascii.push_str("endloop\nendfacet\n");
    }
    ascii.push_str("endsolid fuzz\n");
    vec![binary, ascii.into_bytes()]
}

fn scene_seed() -> Vec<u8> {
    br#"{"schema":"ketchup.sketchup-scene.v1","units":"inch","definitions":[{"id":"d1","name":"Tetra","vertices":[[0,0,0],[1,0,0],[0,1,0],[0,0,1]],"triangles":[[0,2,1],[0,1,3],[0,3,2],[1,2,3]]}],"instances":[{"definition":"d1","name":"Left","transform":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],"visible":true},{"definition":"d1","name":"Right","transform":[1,0,0,2,0,1,0,0,0,0,1,0,0,0,0,2],"visible":true}],"metadata":{"material_assignments":3,"textures":1,"tags":2,"scenes":1,"unsupported_entities":4}}"#.to_vec()
}

#[test]
fn mutated_dxf_files_are_read_or_refused_quickly_without_panicking() {
    for seed in dxf_seeds() {
        if let Err(error) = inspect_dxf(&seed, DxfImportOptions::new(None)) {
            panic!("the seed itself is readable: {error:?}");
        }
    }
    fuzz("dxf", &dxf_seeds(), 600, |bytes| {
        let _ = inspect_dxf(
            bytes,
            DxfImportOptions::new(Some(ImportLengthUnit::Millimetre)),
        );
    });
}

#[test]
fn mutated_stl_files_are_read_or_refused_quickly_without_panicking() {
    for seed in stl_seeds() {
        let units = ImportUnitDecision::new(
            ImportLengthUnit::Millimetre,
            ImportUnitAuthority::UserDeclared,
        );
        assert!(
            parse_stl(&seed, units).is_ok(),
            "the seed itself is readable"
        );
    }
    fuzz("stl", &stl_seeds(), 600, |bytes| {
        let units = ImportUnitDecision::new(
            ImportLengthUnit::Millimetre,
            ImportUnitAuthority::UserDeclared,
        );
        let _ = parse_stl(bytes, units);
    });
}

#[test]
fn mutated_sketchup_scenes_are_read_or_refused_quickly_without_panicking() {
    assert!(
        inspect_sketchup_scene(&scene_seed()).is_ok(),
        "the seed itself is readable"
    );
    fuzz("kscene", &[scene_seed()], 600, |bytes| {
        let _ = inspect_sketchup_scene(bytes);
    });
}

/// The vertex buffer of the GLB seed; the fuzzer mutates the JSON chunk the
/// reader interprets and packs it back with this buffer.
static GLB_BUFFER: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();

fn glb_with_json(json: &[u8]) -> Vec<u8> {
    let buffer = GLB_BUFFER.get().expect("the seed was split first");
    let mut json = json.to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + buffer.len();
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&u32::try_from(total).unwrap().to_le_bytes());
    glb.extend_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    glb.extend_from_slice(buffer);
    glb
}

#[test]
fn mutated_glb_files_are_read_or_refused_quickly_without_panicking() {
    let seed = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../artifacts/blender/garden-studio-colored.glb"
    ))
    .unwrap();
    let json_length =
        usize::try_from(u32::from_le_bytes(seed[12..16].try_into().unwrap())).unwrap();
    assert_eq!(&seed[16..20], b"JSON");
    let json = seed[20..20 + json_length].to_vec();
    GLB_BUFFER.get_or_init(|| seed[20 + json_length..].to_vec());
    assert_eq!(glb_with_json(&json), seed, "the seed packs back unchanged");
    assert!(inspect_glb(&seed).is_ok(), "the seed itself is readable");
    fuzz("glb", &[json], 300, |json| {
        let _ = inspect_glb(&glb_with_json(json));
    });
}
