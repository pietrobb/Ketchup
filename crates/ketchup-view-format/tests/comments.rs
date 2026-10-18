use crate::packages::{camera, encode_glb, fixture, path};
use ketchup_view_format::*;

fn package() -> Package {
    let (manifest, doc, binary) = fixture();
    Package {
        manifest,
        glb: encode_glb(&doc, &binary),
    }
}

fn comment(id: u64) -> Comment {
    Comment {
        id,
        author: Some("Zákazník".into()),
        text: "Posunúť poličku o 2 cm vyššie — mäkčeň ok".into(),
        view: CommentView {
            camera: camera(),
            hidden: vec![path(11)],
            style: DisplayStyle::Wireframe,
            section: Some(Section {
                normal: [0.0, 0.0, 1.0],
                offset_mm: 5.0,
            }),
            scene: None,
            shown: None,
        },
        selected: Some(path(10)),
        arrows: vec![Arrow {
            tip: Anchor {
                occurrence: Some(path(10)),
                point_mm: [5.0, 1.0, 0.0],
            },
            tail: Anchor {
                occurrence: Some(path(10)),
                point_mm: [25.0, 20.0, 15.0],
            },
        }],
    }
}

#[test]
fn comments_roundtrip_offline_and_bind_to_their_export() {
    let package = package();
    let mut comments = Comments::new(&package).expect("new");
    comments.comments.push(comment(1));
    let id = comments.new_id();
    assert!(id != 1 && id < 1 << 53, "{id}");
    comments.comments.push(comment(id));
    let mut bytes = Vec::new();
    comments.write(&mut bytes, &package).expect("write");
    let reopened = Comments::read(bytes.as_slice(), &package).expect("read");
    assert_eq!(
        reopened, comments,
        "text, view, selection and arrows survive"
    );

    // The package checksum the envelope carries is the export identity.
    let mut file = Vec::new();
    package.write(&mut file).expect("package");
    let digest: String = file[21..53].iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(comments.export.package_sha256, digest);

    let mut other = package.clone();
    other.manifest.definitions[0].note = Some("re-exported".into());
    let refused = Comments::read(bytes.as_slice(), &other).expect_err("another export");
    assert!(
        refused.to_string().contains("different export"),
        "{refused}"
    );
}

#[test]
fn commented_copy_keeps_the_export_and_carries_its_comments() {
    let package = package();
    let mut comments = Comments::new(&package).expect("new");
    comments.comments.push(comment(1));
    let mut plain = Vec::new();
    package.write(&mut plain).expect("package");
    let mut copy = Vec::new();
    package
        .write_with_comments(&mut copy, &comments)
        .expect("copy");
    assert_eq!(
        &copy[..plain.len()],
        plain.as_slice(),
        "the export bytes are unchanged"
    );

    let (reopened, carried) = Package::read_with_comments(copy.as_slice()).expect("read copy");
    assert_eq!(reopened, package);
    assert_eq!(carried, Some(comments.clone()));
    assert_eq!(Package::read(copy.as_slice()).expect("plain read"), package);
    let (_, none) = Package::read_with_comments(plain.as_slice()).expect("plain");
    assert_eq!(none, None);

    // A damaged or foreign trailer is never read as comments.
    let mut damaged = copy.clone();
    *damaged.last_mut().unwrap() ^= 1;
    assert!(Package::read_with_comments(damaged.as_slice()).is_err());
    let mut junk = plain.clone();
    junk.extend_from_slice(b"not comments at all, just trailing bytes of some size");
    let refused = Package::read_with_comments(junk.as_slice()).expect_err("junk");
    assert!(refused.to_string().contains("Trailing"), "{refused}");
    let mut truncated = copy;
    truncated.pop();
    assert!(Package::read_with_comments(truncated.as_slice()).is_err());
}

#[test]
fn malformed_oversized_or_dangling_comments_are_refused() {
    let package = package();
    let base = Comments::new(&package).expect("new");
    let cases: Vec<fn(&mut Comment)> = vec![
        |c| c.text = "   ".into(),
        |c| c.text = "x".repeat(MAX_COMMENT_TEXT_BYTES + 1),
        |c| c.author = Some("a\0b".into()),
        |c| c.selected = Some(path(99)),
        |c| c.view.hidden = vec![path(99)],
        |c| c.view.shown = Some(vec![path(99)]),
        |c| c.view.scene = Some(99),
        |c| c.arrows[0].tip.occurrence = Some(path(99)),
        |c| c.arrows[0].tail.point_mm[0] = f64::NAN,
        |c| c.arrows = vec![c.arrows[0].clone(); MAX_ARROWS + 1],
        |c| c.view.section.as_mut().unwrap().normal = [0.0; 3],
        |c| c.id = 0,
    ];
    for (i, change) in cases.into_iter().enumerate() {
        let mut comments = base.clone();
        let mut bad = comment(1);
        change(&mut bad);
        comments.comments.push(bad);
        let mut bytes = Vec::new();
        assert!(comments.write(&mut bytes, &package).is_err(), "case {i}");
        assert!(bytes.is_empty(), "nothing written for case {i}");
    }
    let mut duplicate = base.clone();
    duplicate.comments = vec![comment(1), comment(1)];
    assert!(duplicate.write(&mut Vec::new(), &package).is_err());

    assert!(Comments::read(&b"{}"[..], &package).is_err());
    let huge = vec![b' '; MAX_COMMENTS_BYTES + 1];
    let refused = Comments::read(huge.as_slice(), &package).expect_err("too large");
    assert!(refused.to_string().contains("4 MiB"), "{refused}");
    let mut future = base;
    future.version = 2;
    let bytes = serde_json::to_vec(&future).unwrap();
    assert!(Comments::read(bytes.as_slice(), &package).is_err());
}

/// A house: 6000 loose parts, a wall panel assembly of 400 and a roof layer
/// of every third loose part, hidden by the scene "Without roof".
fn house() -> Package {
    let mut package = package();
    let manifest = &mut package.manifest;
    let part = manifest.occurrences[0].clone();
    let panel = (0..400).map(|local_id| InstancePath {
        root_occurrence_id: 50,
        steps: vec![PathStep::Occurrence {
            owner_definition_id: 1,
            local_id,
        }],
    });
    manifest.occurrences = (1_000..7_000)
        .map(path)
        .chain(panel)
        .map(|path| Occurrence {
            path,
            ..part.clone()
        })
        .collect();
    manifest.scenes[0].id = 7;
    manifest.scenes[0].hidden = (1_000..7_000).step_by(3).map(path).collect();
    manifest.start_scene = Some(7);
    package
}

#[test]
fn scattered_hidden_parts_of_a_hundred_thousand_check_and_restore_at_once() {
    // Every other part hidden by the scene, every third by the comment: no
    // assembly covers them, so each is its own path.
    let mut package = package();
    let manifest = &mut package.manifest;
    let part = manifest.occurrences[0].clone();
    manifest.occurrences = (1_000..101_000)
        .map(|i| Occurrence {
            path: path(i),
            ..part.clone()
        })
        .collect();
    manifest.scenes[0].id = 7;
    manifest.scenes[0].hidden = (1_000..101_000).step_by(2).map(path).collect();
    let mut comments = Comments::new(&package).expect("new");
    let mut written = comment(comments.new_id());
    written.view.scene = Some(7);
    written.view.hidden = (1_000..101_000).step_by(3).map(path).collect();
    written.selected = Some(path(1_001));
    for anchor in written
        .arrows
        .iter_mut()
        .flat_map(|a| [&mut a.tip, &mut a.tail])
    {
        anchor.occurrence = Some(path(1_001));
    }
    comments.comments.push(written);

    let mut bytes = Vec::new();
    comments.write(&mut bytes, &package).expect("write");
    let read = Comments::read(bytes.as_slice(), &package).expect("read");
    let visible = read.comments[0].view.visibility(&package.manifest);

    assert!(
        visible
            .iter()
            .enumerate()
            .all(|(i, shown)| *shown == (i % 2 == 1 && i % 3 != 0)),
        "the comment shows exactly the parts neither it nor its scene hides"
    );
}

/// The parts above in release, nightly: checking each hidden path against
/// every part takes minutes there.
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_scattered_hidden_parts_of_a_hundred_thousand_check_in_seconds() {
    let started = std::time::Instant::now();
    scattered_hidden_parts_of_a_hundred_thousand_check_and_restore_at_once();
    let elapsed = started.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(10), "{elapsed:?}");
}

#[test]
fn isolations_and_hidden_layers_stay_small_and_restore_exactly() {
    let package = house();
    let manifest = &package.manifest;
    let count = manifest.occurrences.len();
    let roof = |i: usize| i < 6_000 && i.is_multiple_of(3);
    let panel = |i: usize| i >= 6_000;
    let views: [(&str, Vec<bool>); 4] = [
        ("isolated part", (0..count).map(|i| i == 4_321).collect()),
        ("roof layer hidden", (0..count).map(|i| !roof(i)).collect()),
        (
            "roof layer, the wall panel and one more part hidden",
            (0..count)
                .map(|i| !roof(i) && !panel(i) && i != 1)
                .collect(),
        ),
        ("wall panel hidden", (0..count).map(|i| !panel(i)).collect()),
    ];
    let mut comments = Comments::new(&package).expect("new");
    for (name, visible) in &views {
        let mut view = comment(1).view;
        view.set_visibility(manifest, visible);
        let restored = view.visibility(manifest);
        assert!(restored == *visible, "{name}: the view restores exactly");
        let paths = view.hidden.len() + view.shown.as_ref().map_or(0, Vec::len);
        assert!(paths <= 2, "{name}: {paths} paths, {view:?}");
        for _ in 0..250 {
            let mut written = comment(comments.new_id());
            written.view = view.clone();
            written.selected = Some(path(5_321));
            for anchor in written
                .arrows
                .iter_mut()
                .flat_map(|a| [&mut a.tip, &mut a.tail])
            {
                anchor.occurrence = Some(path(5_321));
            }
            comments.comments.push(written);
        }
    }
    assert_eq!(
        comments.comments[750].view.hidden,
        [path(50)],
        "a hidden assembly is one path"
    );

    // A thousand comments in these views still save, and read back the same.
    let mut bytes = Vec::new();
    comments.write(&mut bytes, &package).expect("write");
    assert!(
        bytes.len() < MAX_COMMENTS_BYTES / 4,
        "{} bytes",
        bytes.len()
    );
    assert_eq!(
        Comments::read(bytes.as_slice(), &package).expect("read"),
        comments
    );

    // A view a plain list describes as well is written as before, for
    // Viewers that read only `hidden`.
    let mut plain = comment(1).view;
    plain.set_visibility(manifest, &(0..count).map(|i| i != 1).collect::<Vec<_>>());
    assert_eq!(
        (plain.hidden, plain.scene, plain.shown),
        (vec![path(1_001)], None, None)
    );
    let json = serde_json::to_string(&comment(1)).unwrap();
    assert!(!json.contains("shown") && !json.contains("scene"), "{json}");
}
