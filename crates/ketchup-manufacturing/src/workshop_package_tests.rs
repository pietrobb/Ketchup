use super::*;

fn package() -> WorkshopPackage {
    WorkshopPackage {
        files: vec![
            ("btlx/model.btlx".to_owned(), b"<BTLx/>".to_vec()),
            ("bom.txt".to_owned(), b"row=1;quantity=2\n".to_vec()),
        ],
        not_made: vec![NotMade {
            output: "woodWOP".to_owned(),
            reason: Arc::new(GeneralFabricationError::NoPieceToProgram),
        }],
        losses: vec![Lost {
            output: "BTLx".to_owned(),
            part: "Post, \"A\"".to_owned(),
            element: "a tapped hole (M8)".to_owned(),
            reason: "no tapping".to_owned(),
        }],
    }
}

/// Review 2026-11-10 MF-400, MF-401: what the made files lose is a list a
/// program reads, with the part and the element, not lost in the package.
#[test]
fn the_package_lists_what_its_files_lose_as_csv() {
    let entries = stored_entries(&package().zip().unwrap()).unwrap();
    assert_eq!(
        String::from_utf8(entries[WORKSHOP_LOSSES].clone()).unwrap(),
        "output,part,element,reason\n\"BTLx\",\"Post, \"\"A\"\"\",\"a tapped hole (M8)\",\"no tapping\"\n"
    );
    // Review 2026-11-20 ac P3: a part named as a formula is not run as one.
    let mut formula = package();
    formula.losses[0].part = "=HYPERLINK(\"http://x\")".to_owned();
    let entries = stored_entries(&formula.zip().unwrap()).unwrap();
    let csv = String::from_utf8(entries[WORKSHOP_LOSSES].clone()).unwrap();
    assert!(csv.contains(",\"'=HYPERLINK(\"\"http://x\"\")\","), "{csv}");
    let mut lossless = package();
    lossless.losses.clear();
    let entries = stored_entries(&lossless.zip().unwrap()).unwrap();
    assert_eq!(entries[WORKSHOP_LOSSES], b"output,part,element,reason\n");
}

/// Review 2026-11-10 MF-406: when nothing can be made, the refusal names
/// every output with its reason instead of "no shop file".
#[test]
fn a_package_of_nothing_made_is_refused_with_every_reason() {
    let mut nothing = package();
    nothing.files.clear();
    let reason = GeneralFabricationError::NoPieceToProgram.to_string();
    assert_eq!(
        nothing.zip(),
        Err(WorkshopPackageError::NothingMade(vec![format!(
            "woodWOP: {reason}"
        )]))
    );
    nothing.not_made.clear();
    assert_eq!(nothing.zip(), Err(WorkshopPackageError::Empty));
}

#[test]
fn the_workshop_accepts_the_package_and_reads_what_was_not_made() {
    let zip = package().zip().unwrap();
    let entries = stored_entries(&zip).unwrap();
    let sums = String::from_utf8(entries[WORKSHOP_SUMS].clone()).unwrap();
    // `sha256sum -c SHA256SUMS` checks the unpacked package without Kečup.
    assert!(
        sums.contains(&format!("{}  btlx/model.btlx\n", sha256_hex(b"<BTLx/>"))),
        "{sums}"
    );
    let accepted = check_workshop_package(&zip).unwrap();
    assert_eq!(
        accepted.files,
        [
            "btlx/model.btlx",
            "bom.txt",
            WORKSHOP_REPORT,
            WORKSHOP_LOSSES
        ]
    );
    let reason = GeneralFabricationError::NoPieceToProgram.to_string();
    assert!(
        accepted.not_made.contains(&format!("woodWOP: {reason}")),
        "{}",
        accepted.not_made
    );
}

#[test]
fn a_package_with_everything_made_says_so() {
    let mut everything = package();
    everything.not_made.clear();
    let accepted = check_workshop_package(&everything.zip().unwrap()).unwrap();
    assert_eq!(accepted.not_made, "Every output was made.\n");
}

#[test]
fn a_file_changed_on_the_way_fails_the_check() {
    let zip = package().zip().unwrap();
    let mut entries = stored_entries(&zip).unwrap();
    entries.insert("bom.txt".to_owned(), b"row=1;quantity=3\n".to_vec());
    let repacked = repack(&entries);
    assert_eq!(
        check_workshop_package(&repacked),
        Err(WorkshopPackageError::Changed("bom.txt".to_owned()))
    );
}

#[test]
fn a_missing_or_an_extra_file_fails_the_check() {
    let zip = package().zip().unwrap();
    let mut missing = stored_entries(&zip).unwrap();
    missing.remove("bom.txt");
    assert_eq!(
        check_workshop_package(&repack(&missing)),
        Err(WorkshopPackageError::Missing("bom.txt".to_owned()))
    );
    let mut extra = stored_entries(&zip).unwrap();
    extra.insert("run.bat".to_owned(), b"format c:".to_vec());
    assert_eq!(
        check_workshop_package(&repack(&extra)),
        Err(WorkshopPackageError::Unlisted("run.bat".to_owned()))
    );
    let mut no_sums = stored_entries(&zip).unwrap();
    no_sums.remove(WORKSHOP_SUMS);
    assert_eq!(
        check_workshop_package(&repack(&no_sums)),
        Err(WorkshopPackageError::NoSums)
    );
}

#[test]
fn a_damaged_archive_is_not_a_package() {
    let mut zip = package().zip().unwrap();
    let at = zip
        .windows(7)
        .position(|window| window == b"<BTLx/>")
        .unwrap();
    zip[at] = b'!';
    assert_eq!(
        check_workshop_package(&zip),
        Err(WorkshopPackageError::NotAPackage)
    );
    assert_eq!(
        check_workshop_package(b"PK\x05\x06"),
        Err(WorkshopPackageError::NotAPackage)
    );
    assert_eq!(
        check_workshop_package(&[]),
        Err(WorkshopPackageError::NotAPackage)
    );
}

#[test]
fn a_name_that_leaves_the_package_folder_is_refused() {
    for name in [
        "../bom.txt",
        "/bom.txt",
        "a/../../bom.txt",
        "C:/bom.txt",
        "btlx\\model.btlx",
        "",
        "btlx/",
        WORKSHOP_SUMS,
        WORKSHOP_REPORT,
        WORKSHOP_LOSSES,
    ] {
        let mut bad = package();
        bad.files[0].0 = name.to_owned();
        assert_eq!(
            bad.zip(),
            Err(WorkshopPackageError::BadName(name.to_owned())),
            "{name}"
        );
    }
    let mut twice = package();
    twice.files[1].0 = twice.files[0].0.clone();
    assert_eq!(
        twice.zip(),
        Err(WorkshopPackageError::DuplicateName(
            "btlx/model.btlx".to_owned()
        ))
    );
    let empty = WorkshopPackage::default();
    assert_eq!(empty.zip(), Err(WorkshopPackageError::Empty));
}

#[test]
fn the_check_refuses_an_entry_named_outside_the_folder() {
    let zip = package().zip().unwrap();
    let mut entries = stored_entries(&zip).unwrap();
    let bom = entries.remove("bom.txt").unwrap();
    entries.insert("../bom.txt".to_owned(), bom);
    assert_eq!(
        check_workshop_package(&repack(&entries)),
        Err(WorkshopPackageError::BadName("../bom.txt".to_owned()))
    );
}

#[test]
fn a_name_windows_cannot_write_or_that_clashes_there_is_refused() {
    for name in [
        "CON.dxf",
        "btlx/aux",
        "lpt1.txt",
        "com\u{b9}.txt",
        "a<b.txt",
        "what?.txt",
        "name.",
        "btlx /model.btlx",
    ] {
        let mut bad = package();
        bad.files[0].0 = name.to_owned();
        assert_eq!(
            bad.zip(),
            Err(WorkshopPackageError::BadName(name.to_owned())),
            "{name}"
        );
        let zip = package().zip().unwrap();
        let mut entries = stored_entries(&zip).unwrap();
        let bom = entries.remove("bom.txt").unwrap();
        entries.insert(name.to_owned(), bom);
        assert_eq!(
            check_workshop_package(&repack(&entries)),
            Err(WorkshopPackageError::BadName(name.to_owned())),
            "{name}"
        );
    }
    // The same file on Windows and macOS: case and the two Unicode forms.
    for (first, second) in [
        ("bom.txt", "BOM.txt"),
        ("caf\u{e9}.txt", "cafe\u{301}.txt"),
        ("drawings.svg", "sha256sums"),
    ] {
        let mut twice = package();
        twice.files[0].0 = first.to_owned();
        twice.files[1].0 = second.to_owned();
        assert_eq!(
            twice.zip(),
            Err(WorkshopPackageError::DuplicateName(second.to_owned())),
            "{second}"
        );
    }
    let zip = package().zip().unwrap();
    let mut entries = stored_entries(&zip).unwrap();
    entries.insert("BOM.txt".to_owned(), b"row=1;quantity=9\n".to_vec());
    assert_eq!(
        check_workshop_package(&repack(&entries)),
        Err(WorkshopPackageError::DuplicateName("bom.txt".to_owned()))
    );
}

#[test]
fn a_local_header_that_differs_from_the_file_list_fails_the_check() {
    let mut named = package();
    named.files[1].0 = "aaaaa/evil.csv".to_owned();
    let mut zip = named.zip().unwrap();
    // An unpacker that reads the local headers writes `../../evil.csv`.
    let at = zip
        .windows(14)
        .position(|window| window == b"aaaaa/evil.csv")
        .unwrap();
    zip[at..at + 14].copy_from_slice(b"../../evil.csv");
    assert_eq!(
        check_workshop_package(&zip),
        Err(WorkshopPackageError::LocalHeaderDiffers(
            "aaaaa/evil.csv".to_owned()
        ))
    );
}

#[test]
fn a_local_entry_the_file_list_does_not_name_fails_the_check() {
    let mut zip = package().zip().unwrap();
    hide_from_central_directory(&mut zip, "bom.txt");
    assert_eq!(
        check_workshop_package(&zip),
        Err(WorkshopPackageError::HiddenData)
    );
}

#[test]
fn a_name_that_is_not_ascii_is_marked_as_utf8() {
    let mut named = package();
    named.files[1].0 = "drawings/\u{10d}elo.svg".to_owned();
    let zip = named.zip().unwrap();
    let flags = |at: usize| u16::from_le_bytes([zip[at], zip[at + 1]]);
    let mut utf8 = Vec::new();
    for (signature, flags_at) in [(b"PK\x03\x04", 6), (b"PK\x01\x02", 8)] {
        let mut from = 0;
        while let Some(at) = zip[from..].windows(4).position(|w| w == signature) {
            utf8.push(flags(from + at + flags_at) & (1 << 11) != 0);
            from += at + 4;
        }
    }
    // Local and central record of the one non-ASCII name; the ASCII names
    // keep the flag clear.
    assert_eq!(utf8.iter().filter(|&&set| set).count(), 2, "{utf8:?}");
    assert!(check_workshop_package(&zip).is_ok());
    let mut unmarked = zip.clone();
    for at in 0..unmarked.len() - 4 {
        let flags_at = match &unmarked[at..at + 4] {
            b"PK\x03\x04" => at + 6,
            b"PK\x01\x02" => at + 8,
            _ => continue,
        };
        unmarked[flags_at + 1] &= !(1 << 3);
    }
    assert_eq!(
        check_workshop_package(&unmarked),
        Err(WorkshopPackageError::NameNotMarkedUtf8(
            "drawings/\u{10d}elo.svg".to_owned()
        ))
    );
}

/// What one entry's headers say beyond its name, size and CRC.
struct Header {
    flags: u16,
    /// The extra field, the same in the local and the central header.
    extra: Vec<u8>,
    made_by: u16,
    external: u32,
}

/// What Kečup writes for `name`.
fn plain(name: &str) -> Header {
    Header {
        flags: if name.is_ascii() {
            0
        } else {
            crate::three_mf_export::UTF8_NAME
        },
        extra: Vec::new(),
        made_by: 20,
        external: 0,
    }
}

/// A stored ZIP of `entries`, each with the headers `header` gives it.
fn rezip(entries: &BTreeMap<String, Vec<u8>>, header: impl Fn(&str) -> Header) -> Vec<u8> {
    let mut zip = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let header = header(name);
        let offset = u32::try_from(zip.len()).unwrap();
        let crc = crate::three_mf_export::crc32(data);
        let size = u32::try_from(data.len()).unwrap();
        let name_len = u16::try_from(name.len()).unwrap();
        let extra_len = u16::try_from(header.extra.len()).unwrap();
        zip.extend(b"PK\x03\x04");
        for field in [20, header.flags, 0, 0, 0] {
            zip.extend(field.to_le_bytes());
        }
        for field in [crc, size, size] {
            zip.extend(field.to_le_bytes());
        }
        for field in [name_len, extra_len] {
            zip.extend(field.to_le_bytes());
        }
        zip.extend(name.as_bytes());
        zip.extend(&header.extra);
        zip.extend(data);
        central.extend(b"PK\x01\x02");
        for field in [header.made_by, 20, header.flags, 0, 0, 0] {
            central.extend(field.to_le_bytes());
        }
        for field in [crc, size, size] {
            central.extend(field.to_le_bytes());
        }
        for field in [name_len, extra_len, 0, 0, 0] {
            central.extend(field.to_le_bytes());
        }
        for field in [header.external, offset] {
            central.extend(field.to_le_bytes());
        }
        central.extend(name.as_bytes());
        central.extend(&header.extra);
    }
    let central_offset = u32::try_from(zip.len()).unwrap();
    let central_size = u32::try_from(central.len()).unwrap();
    let count = u16::try_from(entries.len()).unwrap();
    zip.extend(central);
    zip.extend(b"PK\x05\x06");
    for field in [0, 0, count, count] {
        zip.extend(field.to_le_bytes());
    }
    for field in [central_size, central_offset] {
        zip.extend(field.to_le_bytes());
    }
    zip.extend(0_u16.to_le_bytes());
    zip
}

/// The entries of the sample package with the two programs `a.nc` and `b.nc`.
fn two_programs() -> BTreeMap<String, Vec<u8>> {
    let mut programs = package();
    programs.files = vec![
        ("a.nc".to_owned(), b"G0 X1\n".to_vec()),
        ("b.nc".to_owned(), b"G0 X2\n".to_vec()),
    ];
    stored_entries(&programs.zip().unwrap()).unwrap()
}

/// The Info-ZIP Unicode Path extra field naming `unicode` for the header `name`.
fn unicode_path(name: &str, unicode: &str) -> Vec<u8> {
    let mut field = 0x7075_u16.to_le_bytes().to_vec();
    field.extend(u16::try_from(5 + unicode.len()).unwrap().to_le_bytes());
    field.push(1);
    field.extend(crate::three_mf_export::crc32(name.as_bytes()).to_le_bytes());
    field.extend(unicode.as_bytes());
    field
}

/// R1120ac-ZIP-900: an extra field, such as the Unicode Path an unpacker takes for
/// the real name, is refused: `a.nc` and `b.nc` naming each other there unpacked each
/// as the other, and `../../evil.nc` passed. The same headers without it pass.
#[test]
fn an_extra_field_an_unpacker_may_take_the_name_from_is_refused() {
    let entries = two_programs();
    assert!(check_workshop_package(&rezip(&entries, plain)).is_ok());
    for (other, swapped) in [("b.nc", true), ("../../evil.nc", false)] {
        let zip = rezip(&entries, |name| {
            let mut header = plain(name);
            if name == "a.nc" {
                header.extra = unicode_path(name, other);
            } else if name == "b.nc" && swapped {
                header.extra = unicode_path(name, "a.nc");
            }
            header
        });
        assert_eq!(
            check_workshop_package(&zip),
            Err(WorkshopPackageError::ExtraField("a.nc".to_owned())),
            "{other}"
        );
    }
}

/// R1120ac-ZIP-901: an entry made on Unix whose mode is not a plain file, such as a
/// link `a.nc -> /etc/hostname`, is refused; a plain Unix file passes.
#[test]
fn a_link_or_other_special_file_is_refused() {
    let entries = two_programs();
    let unix = |mode: u32| {
        move |name: &str| {
            let mut header = plain(name);
            if name == "a.nc" {
                header.made_by = (3 << 8) | 20;
                header.external = mode << 16;
            }
            header
        }
    };
    assert!(check_workshop_package(&rezip(&entries, unix(0o100_644))).is_ok());
    for mode in [0o120_777, 0o040_755, 0o010_644] {
        assert_eq!(
            check_workshop_package(&rezip(&entries, unix(mode))),
            Err(WorkshopPackageError::NotAPlainFile("a.nc".to_owned())),
            "{mode:o}"
        );
    }
    // A DOS directory or a device is no plain file either.
    let dos_directory = rezip(&entries, |name| Header {
        external: if name == "a.nc" { 0x10 } else { 0 },
        ..plain(name)
    });
    assert_eq!(
        check_workshop_package(&dos_directory),
        Err(WorkshopPackageError::NotAPlainFile("a.nc".to_owned()))
    );
}

/// Review 2026-11-20 ac P3: a header flag other than UTF-8 (bit 11), such as a data
/// descriptor (bit 3) with none after the data or strong encryption (bit 6), is
/// refused, even when both headers agree.
#[test]
fn a_header_flag_other_than_utf8_is_refused() {
    let entries = two_programs();
    for flag in [1_u16 << 3, 1 << 6, 1 << 13] {
        let zip = rezip(&entries, |name| Header {
            flags: if name == "a.nc" { flag } else { 0 },
            ..plain(name)
        });
        assert_eq!(
            check_workshop_package(&zip),
            Err(WorkshopPackageError::UnknownFlags("a.nc".to_owned())),
            "{flag:#x}"
        );
    }
}

/// Removes the central record of `name`, leaving its local entry in the
/// archive where no file list names it.
fn hide_from_central_directory(zip: &mut Vec<u8>, name: &str) {
    let end = zip.len() - 22;
    let central = zip
        .windows(4)
        .enumerate()
        .position(|(at, window)| {
            window == b"PK\x01\x02" && zip[at + 46..].starts_with(name.as_bytes())
        })
        .unwrap();
    let record = 46 + name.len();
    zip.drain(central..central + record);
    let end = end - record;
    for count_at in [end + 8, end + 10] {
        let count = u16::from_le_bytes([zip[count_at], zip[count_at + 1]]) - 1;
        zip[count_at..count_at + 2].copy_from_slice(&count.to_le_bytes());
    }
    let size_at = end + 12;
    let size = u32::from_le_bytes(zip[size_at..size_at + 4].try_into().unwrap())
        - u32::try_from(record).unwrap();
    zip[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
}

fn repack(entries: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let entries = entries
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect::<Vec<_>>();
    crate::three_mf_export::encode_package(&entries, usize::MAX).unwrap()
}
