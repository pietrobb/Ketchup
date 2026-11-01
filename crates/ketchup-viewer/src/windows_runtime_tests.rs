//! The portable Viewer runs on a clean Windows: nothing it is linked with
//! needs the Visual C++ runtime DLLs, which a clean Windows does not have
//! (VW-81). This test binary is linked as the release Viewer is: with
//! `-C target-feature=+crt-static` and the hybrid CRT of build.rs. CI runs
//! it so (`.github/workflows/ci.yml`, "the Viewer carries its C++ runtime").

/// The DLLs a PE image imports, directly and delay-loaded, in lower case.
fn imported_dlls(image: &[u8]) -> Vec<String> {
    let u16_at = |at: usize| u16::from_le_bytes([image[at], image[at + 1]]);
    let u32_at =
        |at: usize| u32::from_le_bytes([image[at], image[at + 1], image[at + 2], image[at + 3]]);
    let pe = u32_at(0x3c) as usize;
    assert_eq!(&image[pe..pe + 4], b"PE\0\0", "a PE image");
    let sections = usize::from(u16_at(pe + 6));
    let optional = pe + 24;
    let optional_size = usize::from(u16_at(pe + 20));
    assert_eq!(u16_at(optional), 0x20b, "a PE32+ image");
    let section_table = optional + optional_size;
    let offset_of = |rva: u32| {
        (0..sections)
            .map(|index| section_table + index * 40)
            .find_map(|header| {
                let start = u32_at(header + 12);
                let size = u32_at(header + 8).max(u32_at(header + 16));
                (start..start + size)
                    .contains(&rva)
                    .then(|| (rva - start + u32_at(header + 20)) as usize)
            })
            .expect("the RVA lies in a section")
    };
    let name_at = |rva: u32| {
        let start = offset_of(rva);
        let end = start + image[start..].iter().position(|&b| b == 0).expect("a name");
        String::from_utf8_lossy(&image[start..end]).to_ascii_lowercase()
    };
    let mut dlls = Vec::new();
    // Data directory 1 holds the imports (20-byte descriptors, name at +12),
    // 13 the delay-loaded ones (32-byte descriptors, name at +4).
    for (directory, entry, name) in [(1, 20, 12), (13, 32, 4)] {
        let rva = u32_at(optional + 112 + directory * 8);
        if rva == 0 {
            continue;
        }
        let mut at = offset_of(rva);
        while image[at..at + entry].iter().any(|&b| b != 0) {
            dlls.push(name_at(u32_at(at + name)));
            at += entry;
        }
    }
    dlls
}

#[test]
fn the_viewer_needs_no_visual_cpp_runtime_dll() {
    let image = std::fs::read(std::env::current_exe().expect("this test binary")).expect("read");
    let dlls = imported_dlls(&image);
    assert!(
        dlls.iter().any(|dll| dll == "kernel32.dll"),
        "imports are read: {dlls:?}"
    );
    let runtime: Vec<_> = dlls
        .iter()
        .filter(|dll| {
            ["vcruntime", "msvcp", "concrt"]
                .iter()
                .any(|name| dll.starts_with(name))
        })
        .collect();
    assert!(
        runtime.is_empty(),
        "a clean Windows has no {runtime:?}; the Viewer must carry the Visual C++ runtime itself"
    );
}
