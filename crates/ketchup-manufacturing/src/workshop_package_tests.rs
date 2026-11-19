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
    }
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
        ["btlx/model.btlx", "bom.txt", WORKSHOP_REPORT]
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
