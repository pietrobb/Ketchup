use super::*;

fn package() -> WorkshopPackage {
    WorkshopPackage {
        files: vec![
            ("btlx/model.btlx".to_owned(), b"<BTLx/>".to_vec()),
            ("bom.txt".to_owned(), b"row=1;quantity=2\n".to_vec()),
        ],
        not_made: vec![NotMade {
            output: "woodWOP".to_owned(),
            reason: "the part Shelf has a cut no program describes".to_owned(),
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
    assert!(
        accepted
            .not_made
            .contains("woodWOP: the part Shelf has a cut no program describes"),
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

fn repack(entries: &BTreeMap<String, Vec<u8>>) -> Vec<u8> {
    let entries = entries
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect::<Vec<_>>();
    crate::three_mf_export::encode_package(&entries, usize::MAX).unwrap()
}
