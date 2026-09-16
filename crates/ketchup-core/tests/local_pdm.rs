use std::fs;
#[cfg(windows)]
use std::process::Command;

use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, Transform,
};
use ketchup_core::import::{ImportLengthUnit, StepImportEvidence, plan_step_import};
use ketchup_core::local_pdm::{
    DependencyChange, DependencyChangeKind, LocalPdmError, ReleaseAudit, ReleaseConflictVerdict,
    ReleaseDependencyInput, ReleaseRelationship, compare_releases, create_child_release,
    create_child_release_with_container, create_release, create_release_with_container,
    open_release, release_catalog, release_manifest_path, release_object_path,
};
use ketchup_core::persistence;

fn released_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Released bracket".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "Bracket profile".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [80.0, 0.0], [80.0, 40.0], [0.0, 40.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "Bracket body".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(1),
                    height: Dimension::new("12", 12.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Bracket A".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]))
        .unwrap();
    document
}

fn audit() -> ReleaseAudit {
    ReleaseAudit::new("local-user", 1_789_321_000_000, "Approved bracket release")
}

#[cfg(unix)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) {
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[cfg(windows)]
fn create_directory_link(target: &std::path::Path, link: &std::path::Path) {
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()
        .unwrap();
    assert!(status.success(), "directory junction creation failed");
}

fn editable_copy(snapshot: &ketchup_core::document::Snapshot) -> DocumentStore {
    match persistence::load(&persistence::save(snapshot))
        .unwrap()
        .into_editable()
    {
        Ok(document) => document,
        Err(_) => panic!("current release fixture must remain editable"),
    }
}

fn add_branch_revision(document: &mut DocumentStore, name: &str) {
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(2),
                name: name.to_owned(),
            },
        ]))
        .unwrap();
}

#[test]
fn immutable_release_reopens_after_external_sources_change() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let dependency_path = directory.path().join("supplier.step");
    fs::write(&dependency_path, b"ISO-10303-21; original bearing").unwrap();
    let document = released_document();
    let snapshot = document.current();
    let digest = snapshot.canonical_digest();

    let manifest = create_release(
        &repository,
        &snapshot,
        snapshot.revision_id(),
        &digest,
        &[ReleaseDependencyInput::new(
            "supplier/bearing.step",
            &dependency_path,
        )],
        audit(),
    )
    .unwrap();

    fs::write(&dependency_path, b"changed external source").unwrap();
    let verified = open_release(&repository, &manifest.release_id).unwrap();
    assert_eq!(verified.manifest, manifest);
    assert_eq!(verified.snapshot.document_id(), snapshot.document_id());
    assert_eq!(verified.snapshot.revision_id(), snapshot.revision_id());
    assert_eq!(verified.snapshot.canonical_digest(), digest);
    assert_eq!(verified.dependency_objects.len(), 1);
    assert_eq!(
        verified.dependency_objects["supplier/bearing.step"],
        manifest.dependencies[0].object
    );
    let object_path = release_object_path(
        &repository,
        &verified.dependency_objects["supplier/bearing.step"].sha256,
    )
    .unwrap();
    fs::write(&object_path, b"replaced after verification").unwrap();
    assert_eq!(
        verified.dependency_objects["supplier/bearing.step"],
        manifest.dependencies[0].object
    );
    assert!(matches!(
        open_release(&repository, &manifest.release_id),
        Err(LocalPdmError::ObjectTampered { sha256 })
            if sha256 == manifest.dependencies[0].object.sha256
    ));
}

#[test]
fn release_with_imported_exact_geometry_reopens_with_its_source_blob() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let source = b"ISO-10303-21; imported release source";
    let evidence = StepImportEvidence {
        source_sha256: ketchup_core::graph::sha256_bytes(source),
        source_byte_len: source.len() as u64,
        source_unit: ImportLengthUnit::Millimetre,
        result_fingerprint: "0123456789abcdef".to_owned(),
        body_kind: ketchup_core::document::BodyKind::Solid,
        solid_count: 1,
        topology_counts: [8, 12, 6, 1, 1],
        area_mm2: 600.0,
        volume_mm3: 1_000.0,
        bounds_mm: [[0.0, 0.0, 0.0], [10.0, 10.0, 10.0]],
        backend: "occt-test".to_owned(),
        tolerance: "test-tolerance".to_owned(),
    };
    let mut document = DocumentStore::new();
    let batch = plan_step_import(&document.current(), source, "part.step", &evidence).unwrap();
    document.apply_batch(&batch).unwrap();
    let snapshot = document.current();
    let mut container_data = persistence::ContainerData::default();
    let source_hash = container_data.insert_import_blob(source.to_vec()).unwrap();

    assert!(matches!(
        create_release(
            &repository,
            &snapshot,
            snapshot.revision_id(),
            &snapshot.canonical_digest(),
            &[],
            audit(),
        ),
        Err(LocalPdmError::Persistence(
            persistence::PersistenceError::InvalidBlobHash
        ))
    ));
    assert!(!repository.exists());

    let manifest = create_release_with_container(
        &repository,
        (&snapshot, &container_data),
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[],
        audit(),
    )
    .unwrap();

    let reopened = open_release(&repository, &manifest.release_id).unwrap();
    assert_eq!(
        reopened.snapshot.canonical_digest(),
        snapshot.canonical_digest()
    );
    assert_eq!(
        reopened
            .container_data
            .blobs()
            .get(&source_hash)
            .map(Vec::as_slice),
        Some(source.as_slice())
    );

    add_branch_revision(&mut document, "Released imported child");
    let child_snapshot = document.current();
    let child = create_child_release_with_container(
        &repository,
        &manifest.release_id,
        (&child_snapshot, &container_data),
        child_snapshot.revision_id(),
        &child_snapshot.canonical_digest(),
        &[],
        audit(),
    )
    .unwrap();
    let reopened_child = open_release(&repository, &child.release_id).unwrap();
    assert_eq!(
        reopened_child
            .container_data
            .blobs()
            .get(&source_hash)
            .map(Vec::as_slice),
        Some(source.as_slice())
    );

    let object_path = release_object_path(&repository, &child.document.object.sha256).unwrap();
    fs::write(&object_path, b"tampered imported release").unwrap();
    assert!(matches!(
        open_release(&repository, &child.release_id),
        Err(LocalPdmError::ObjectTampered { sha256 })
            if sha256 == child.document.object.sha256
    ));
}

#[test]
fn stale_snapshot_is_rejected_before_repository_creation() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let document = released_document();
    let snapshot = document.current();

    assert!(matches!(
        create_release(
            &repository,
            &snapshot,
            snapshot.revision_id() + 1,
            &snapshot.canonical_digest(),
            &[],
            audit(),
        ),
        Err(LocalPdmError::StaleSnapshot)
    ));
    assert!(!repository.exists());

    assert!(matches!(
        create_release(
            &repository,
            &snapshot,
            snapshot.revision_id(),
            "stale-digest",
            &[],
            audit(),
        ),
        Err(LocalPdmError::StaleSnapshot)
    ));
    assert!(!repository.exists());
}

#[test]
fn repository_directory_links_cannot_escape_object_or_manifest_storage() {
    let directory = tempfile::tempdir().unwrap();
    let document = released_document();
    let snapshot = document.current();
    let reference = create_release(
        directory.path().join("reference"),
        &snapshot,
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[],
        audit(),
    )
    .unwrap();

    let object_repository = directory.path().join("object-repository");
    let object_outside = directory.path().join("object-outside");
    fs::create_dir_all(object_repository.join("objects")).unwrap();
    fs::create_dir_all(&object_outside).unwrap();
    create_directory_link(
        &object_outside,
        &object_repository
            .join("objects")
            .join(&reference.document.object.sha256[..2]),
    );
    assert!(matches!(
        create_release(
            &object_repository,
            &snapshot,
            snapshot.revision_id(),
            &snapshot.canonical_digest(),
            &[],
            audit(),
        ),
        Err(LocalPdmError::InvalidRepositoryPath)
    ));
    assert!(fs::read_dir(&object_outside).unwrap().next().is_none());

    let manifest_repository = directory.path().join("manifest-repository");
    let manifest_outside = directory.path().join("manifest-outside");
    fs::create_dir_all(&manifest_repository).unwrap();
    fs::create_dir_all(&manifest_outside).unwrap();
    create_directory_link(&manifest_outside, &manifest_repository.join("releases"));
    assert!(matches!(
        create_release(
            &manifest_repository,
            &snapshot,
            snapshot.revision_id(),
            &snapshot.canonical_digest(),
            &[],
            audit(),
        ),
        Err(LocalPdmError::InvalidRepositoryPath)
    ));
    assert!(fs::read_dir(&manifest_outside).unwrap().next().is_none());

    let object_read_repository = directory.path().join("object-read-repository");
    fs::create_dir_all(object_read_repository.join("releases")).unwrap();
    fs::copy(
        release_manifest_path(directory.path().join("reference"), &reference.release_id).unwrap(),
        release_manifest_path(&object_read_repository, &reference.release_id).unwrap(),
    )
    .unwrap();
    fs::create_dir_all(object_read_repository.join("objects")).unwrap();
    create_directory_link(
        &directory
            .path()
            .join("reference")
            .join("objects")
            .join(&reference.document.object.sha256[..2]),
        &object_read_repository
            .join("objects")
            .join(&reference.document.object.sha256[..2]),
    );
    assert!(matches!(
        open_release(&object_read_repository, &reference.release_id),
        Err(LocalPdmError::InvalidRepositoryPath)
    ));

    let manifest_read_repository = directory.path().join("manifest-read-repository");
    fs::create_dir_all(&manifest_read_repository).unwrap();
    create_directory_link(
        &directory.path().join("reference").join("releases"),
        &manifest_read_repository.join("releases"),
    );
    assert!(matches!(
        open_release(&manifest_read_repository, &reference.release_id),
        Err(LocalPdmError::InvalidRepositoryPath)
    ));
    assert!(matches!(
        release_catalog(&manifest_read_repository),
        Err(LocalPdmError::InvalidRepositoryPath)
    ));
}

#[test]
fn manifest_tampering_is_rejected_by_release_identity() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let document = released_document();
    let snapshot = document.current();
    let manifest = create_release(
        &repository,
        &snapshot,
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[],
        audit(),
    )
    .unwrap();
    let path = release_manifest_path(&repository, &manifest.release_id).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    let offset = bytes
        .windows(b"Approved".len())
        .position(|window| window == b"Approved")
        .unwrap();
    bytes[offset] = b'X';
    fs::write(path, bytes).unwrap();

    assert!(matches!(
        open_release(&repository, &manifest.release_id),
        Err(LocalPdmError::ManifestIdentityMismatch)
    ));
}

#[test]
fn dependency_tampering_and_missing_objects_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let dependency_path = directory.path().join("motor.step");
    fs::write(&dependency_path, b"motor-v1").unwrap();
    let document = released_document();
    let snapshot = document.current();
    let manifest = create_release(
        &repository,
        &snapshot,
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[ReleaseDependencyInput::new(
            "vendor/motor.step",
            &dependency_path,
        )],
        audit(),
    )
    .unwrap();
    let object = &manifest.dependencies[0].object;
    let object_path = release_object_path(&repository, &object.sha256).unwrap();

    fs::write(&object_path, b"motor-v2").unwrap();
    assert!(matches!(
        open_release(&repository, &manifest.release_id),
        Err(LocalPdmError::ObjectTampered { sha256 }) if sha256 == object.sha256
    ));

    fs::remove_file(&object_path).unwrap();
    assert!(matches!(
        open_release(&repository, &manifest.release_id),
        Err(LocalPdmError::MissingObject { sha256 }) if sha256 == object.sha256
    ));
}

#[test]
fn released_document_tampering_fails_before_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let document = released_document();
    let snapshot = document.current();
    let manifest = create_release(
        &repository,
        &snapshot,
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[],
        audit(),
    )
    .unwrap();
    let document_path = release_object_path(&repository, &manifest.document.object.sha256).unwrap();
    let mut bytes = fs::read(&document_path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(document_path, bytes).unwrap();

    assert!(matches!(
        open_release(&repository, &manifest.release_id),
        Err(LocalPdmError::ObjectTampered { sha256 })
            if sha256 == manifest.document.object.sha256
    ));
}

#[test]
fn dependency_paths_are_canonical_unique_and_release_is_never_overwritten() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let dependency_path = directory.path().join("part.step");
    fs::write(&dependency_path, b"part").unwrap();
    let document = released_document();
    let snapshot = document.current();

    assert!(matches!(
        create_release(
            &repository,
            &snapshot,
            snapshot.revision_id(),
            &snapshot.canonical_digest(),
            &[ReleaseDependencyInput::new(
                "../part.step",
                &dependency_path
            )],
            audit(),
        ),
        Err(LocalPdmError::InvalidLogicalPath)
    ));
    assert!(matches!(
        create_release(
            &repository,
            &snapshot,
            snapshot.revision_id(),
            &snapshot.canonical_digest(),
            &[
                ReleaseDependencyInput::new("vendor/part.step", &dependency_path),
                ReleaseDependencyInput::new("vendor/part.step", &dependency_path),
            ],
            audit(),
        ),
        Err(LocalPdmError::DuplicateLogicalPath)
    ));

    let manifest = create_release(
        &repository,
        &snapshot,
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[],
        audit(),
    )
    .unwrap();
    assert!(matches!(
        create_release(
            &repository,
            &snapshot,
            snapshot.revision_id(),
            &snapshot.canonical_digest(),
            &[],
            audit(),
        ),
        Err(LocalPdmError::ReleaseAlreadyExists)
    ));
    assert!(open_release(&repository, &manifest.release_id).is_ok());
}

#[test]
fn catalog_lineage_diff_and_divergence_are_deterministic() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let motor = directory.path().join("motor.step");
    let manual = directory.path().join("manual.pdf");
    let bearing = directory.path().join("bearing.step");
    fs::write(&motor, b"motor-v1").unwrap();
    fs::write(&manual, b"manual-v1").unwrap();
    fs::write(&bearing, b"bearing-v1").unwrap();
    let document = released_document();
    let root_snapshot = document.current();
    let root = create_release(
        &repository,
        &root_snapshot,
        root_snapshot.revision_id(),
        &root_snapshot.canonical_digest(),
        &[
            ReleaseDependencyInput::new("vendor/manual.pdf", &manual),
            ReleaseDependencyInput::new("vendor/motor.step", &motor),
        ],
        ReleaseAudit::new("designer", 100, "Initial approved release"),
    )
    .unwrap();

    let reopened_root = open_release(&repository, &root.release_id).unwrap();
    let mut branch_a_document = editable_copy(&reopened_root.snapshot);
    add_branch_revision(&mut branch_a_document, "Machined variant");
    fs::write(&motor, b"motor-v2").unwrap();
    let branch_a_snapshot = branch_a_document.current();
    let branch_a = create_child_release(
        &repository,
        &root.release_id,
        &branch_a_snapshot,
        branch_a_snapshot.revision_id(),
        &branch_a_snapshot.canonical_digest(),
        &[
            ReleaseDependencyInput::new("vendor/bearing.step", &bearing),
            ReleaseDependencyInput::new("vendor/motor.step", &motor),
        ],
        ReleaseAudit::new("designer", 200, "Machined branch"),
    )
    .unwrap();

    let mut branch_b_document = editable_copy(&reopened_root.snapshot);
    add_branch_revision(&mut branch_b_document, "Cast variant");
    fs::write(&motor, b"motor-v1").unwrap();
    let branch_b_snapshot = branch_b_document.current();
    let branch_b = create_child_release(
        &repository,
        &root.release_id,
        &branch_b_snapshot,
        branch_b_snapshot.revision_id(),
        &branch_b_snapshot.canonical_digest(),
        &[
            ReleaseDependencyInput::new("vendor/manual.pdf", &manual),
            ReleaseDependencyInput::new("vendor/motor.step", &motor),
        ],
        ReleaseAudit::new("reviewer", 300, "Cast branch"),
    )
    .unwrap();

    let forward = compare_releases(&repository, &root.release_id, &branch_a.release_id).unwrap();
    assert_eq!(forward.relationship, ReleaseRelationship::LeftAncestor);
    assert_eq!(
        forward.conflict_verdict,
        ReleaseConflictVerdict::FastForward
    );
    assert!(forward.document_changed);
    assert_eq!(
        forward.dependency_changes,
        vec![
            DependencyChange {
                logical_path: "vendor/bearing.step".to_owned(),
                kind: DependencyChangeKind::Added,
            },
            DependencyChange {
                logical_path: "vendor/manual.pdf".to_owned(),
                kind: DependencyChangeKind::Removed,
            },
            DependencyChange {
                logical_path: "vendor/motor.step".to_owned(),
                kind: DependencyChangeKind::Modified,
            },
        ]
    );

    let divergence =
        compare_releases(&repository, &branch_a.release_id, &branch_b.release_id).unwrap();
    assert_eq!(
        divergence.relationship,
        ReleaseRelationship::Diverged {
            common_ancestor: root.release_id.clone(),
        }
    );
    assert_eq!(
        divergence.conflict_verdict,
        ReleaseConflictVerdict::DivergedConflict
    );
    let reverse = compare_releases(&repository, &branch_a.release_id, &root.release_id).unwrap();
    assert_eq!(reverse.relationship, ReleaseRelationship::RightAncestor);
    assert_eq!(
        reverse.conflict_verdict,
        ReleaseConflictVerdict::IncomingBehind
    );

    let catalog = release_catalog(&repository).unwrap();
    assert_eq!(catalog.len(), 3);
    assert_eq!(catalog[0].release_id, root.release_id);
    assert_eq!(catalog[1].release_id, branch_a.release_id);
    assert_eq!(catalog[2].release_id, branch_b.release_id);
    assert_eq!(catalog[0].parent_release_id, None);
    assert_eq!(catalog[1].parent_release_id, Some(root.release_id));
}

#[test]
fn child_release_rejects_noop_foreign_and_unverifiable_parents() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let document = released_document();
    let snapshot = document.current();
    let root = create_release(
        &repository,
        &snapshot,
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[],
        ReleaseAudit::new("designer", 100, "Root"),
    )
    .unwrap();

    assert!(matches!(
        create_child_release(
            &repository,
            &root.release_id,
            &snapshot,
            snapshot.revision_id(),
            &snapshot.canonical_digest(),
            &[],
            ReleaseAudit::new("designer", 200, "No-op"),
        ),
        Err(LocalPdmError::NoChangesAgainstParent)
    ));

    let other_document = released_document();
    let other_snapshot = other_document.current();
    assert!(matches!(
        create_child_release(
            &repository,
            &root.release_id,
            &other_snapshot,
            other_snapshot.revision_id(),
            &other_snapshot.canonical_digest(),
            &[],
            ReleaseAudit::new("designer", 300, "Foreign"),
        ),
        Err(LocalPdmError::ParentDocumentMismatch)
    ));

    let root_manifest_path = release_manifest_path(&repository, &root.release_id).unwrap();
    fs::remove_file(&root_manifest_path).unwrap();
    let mut branch_document = editable_copy(&snapshot);
    add_branch_revision(&mut branch_document, "Blocked branch");
    let branch_snapshot = branch_document.current();
    assert!(matches!(
        create_child_release(
            &repository,
            &root.release_id,
            &branch_snapshot,
            branch_snapshot.revision_id(),
            &branch_snapshot.canonical_digest(),
            &[],
            ReleaseAudit::new("designer", 400, "Missing parent"),
        ),
        Err(LocalPdmError::MissingRelease { release_id }) if release_id == root.release_id
    ));
}

#[test]
fn lineage_comparison_fails_when_parent_manifest_is_removed() {
    let directory = tempfile::tempdir().unwrap();
    let repository = directory.path().join("pdm");
    let document = released_document();
    let snapshot = document.current();
    let root = create_release(
        &repository,
        &snapshot,
        snapshot.revision_id(),
        &snapshot.canonical_digest(),
        &[],
        ReleaseAudit::new("designer", 100, "Root"),
    )
    .unwrap();
    let mut child_document = editable_copy(&snapshot);
    add_branch_revision(&mut child_document, "Child");
    let child_snapshot = child_document.current();
    let child = create_child_release(
        &repository,
        &root.release_id,
        &child_snapshot,
        child_snapshot.revision_id(),
        &child_snapshot.canonical_digest(),
        &[],
        ReleaseAudit::new("designer", 200, "Child"),
    )
    .unwrap();

    fs::remove_file(release_manifest_path(&repository, &root.release_id).unwrap()).unwrap();
    assert!(matches!(
        release_catalog(&repository),
        Err(LocalPdmError::MissingRelease { release_id }) if release_id == root.release_id
    ));
    assert!(matches!(
        compare_releases(&repository, &child.release_id, &child.release_id),
        Err(LocalPdmError::MissingRelease { release_id }) if release_id == root.release_id
    ));
    assert!(matches!(
        compare_releases(&repository, &child.release_id, &root.release_id),
        Err(LocalPdmError::MissingRelease { release_id }) if release_id == root.release_id
    ));
}
