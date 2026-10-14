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
