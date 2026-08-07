use ed25519_dalek::{Signer, SigningKey};
use ketchup_core::document::{CanonicalCommand, CommandBatch, DefinitionId, DocumentStore};
use ketchup_core::graph::sha256_hex;
use ketchup_core::validation::{
    EvidenceClass, PolicyRequirement, PolicySeverity, ReadScope, ResourceLimits, ValidationClass,
    ValidationInvocation, ValidationPolicyRef, ValidationState, ValidatorDescriptor,
};
use ketchup_core::validator_hosting::{
    HostedValidatorResolution, InstallOutcome, LicenseState, SignedValidatorPackage,
    ValidatorHostingError, ValidatorLicense, ValidatorPackageHost, ValidatorPackageManifest,
    ValidatorRuntime, validator_descriptor_digest,
};

const PUBLISHER: &str = "org.ketchup.tests.publisher";
const PACKAGE: &str = "org.ketchup.tests.validator";

fn signing_key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn descriptor(release: u64) -> ValidatorDescriptor {
    ValidatorDescriptor {
        contract_id: "ketchup.validator.prismatic-joints.v1".to_owned(),
        contract_version: 1,
        implementation_id: "org.ketchup.tests.validator.cpu.v1".to_owned(),
        implementation_version: format!("1.0.{release}"),
        input_schema: "ketchup.prismatic-joint-input.v1".to_owned(),
        validation_class: ValidationClass::DeclaredJoint,
        read_scopes: vec![ReadScope::DerivedGeometry, ReadScope::DeclaredJoints],
        deterministic: true,
        limits: ResourceLimits {
            maximum_input_bytes: 32 * 1024,
            maximum_work_units: 1_000,
        },
    }
}

fn signed_package(
    key: &SigningKey,
    release: u64,
    artifact: &[u8],
    license: ValidatorLicense,
) -> SignedValidatorPackage {
    let descriptor = descriptor(release);
    let manifest = ValidatorPackageManifest::new(
        PACKAGE,
        format!("1.0.{release}"),
        release,
        PUBLISHER,
        &descriptor.contract_id,
        descriptor.contract_version,
        &descriptor.implementation_id,
        &descriptor.implementation_version,
        validator_descriptor_digest(&descriptor),
        sha256_hex(artifact),
        ValidatorRuntime::WasmNoImports,
        vec!["validation.example.test".to_owned()],
        license,
    )
    .unwrap();
    let signature = key.sign(&manifest.signing_payload()).to_bytes();
    SignedValidatorPackage::new(manifest, signature)
}

fn trusted_host(key: &SigningKey) -> ValidatorPackageHost {
    let mut host = ValidatorPackageHost::default();
    host.trust_publisher(PUBLISHER, key.verifying_key().to_bytes())
        .unwrap();
    host
}

fn invocation(store: &DocumentStore, descriptor: &ValidatorDescriptor) -> ValidationInvocation {
    let policy = ValidationPolicyRef {
        policy_id: "ketchup.policy.tests.m7c.v1".to_owned(),
        policy_version: 1,
        contract_id: descriptor.contract_id.clone(),
        contract_version: descriptor.contract_version,
        requirement: PolicyRequirement::Required,
        severity: PolicySeverity::Error,
        blocks_release: true,
        governing_standard: None,
    };
    ValidationInvocation::bind(
        &store.current(),
        descriptor,
        &policy,
        vec![],
        b"bounded input",
    )
}

#[test]
fn m7c_discovers_installs_and_monotonically_updates_a_signed_validator() {
    let key = signing_key(7);
    let mut host = trusted_host(&key);
    let first_artifact = b"bounded validator artifact release one".to_vec();
    assert_eq!(
        host.install(
            signed_package(&key, 1, &first_artifact, ValidatorLicense::OpenSource,),
            first_artifact,
        )
        .unwrap(),
        InstallOutcome::Installed
    );
    assert_eq!(host.discover().len(), 1);
    assert_eq!(host.resolve(PACKAGE).unwrap().manifest().release(), 1);

    let second_artifact = b"bounded validator artifact release two".to_vec();
    assert_eq!(
        host.install(
            signed_package(&key, 2, &second_artifact, ValidatorLicense::OpenSource,),
            second_artifact.clone(),
        )
        .unwrap(),
        InstallOutcome::Updated
    );
    let resolved = host.resolve(PACKAGE).unwrap();
    assert_eq!(resolved.manifest().version(), "1.0.2");
    assert_eq!(resolved.artifact(), second_artifact);

    let rollback_artifact = b"attempted rollback".to_vec();
    assert!(matches!(
        host.install(
            signed_package(&key, 1, &rollback_artifact, ValidatorLicense::OpenSource,),
            rollback_artifact,
        ),
        Err(ValidatorHostingError::ReleaseNotNewer {
            installed: 2,
            candidate: 1
        })
    ));
}

#[test]
fn m7c_rejects_untrusted_signatures_and_artifact_tampering() {
    let trusted_key = signing_key(11);
    let untrusted_key = signing_key(12);
    let artifact = b"authentic artifact".to_vec();

    let mut no_trust = ValidatorPackageHost::default();
    assert!(matches!(
        no_trust.install(
            signed_package(&trusted_key, 1, &artifact, ValidatorLicense::OpenSource,),
            artifact.clone(),
        ),
        Err(ValidatorHostingError::PublisherUntrusted)
    ));

    let mut wrong_signature = trusted_host(&trusted_key);
    assert!(matches!(
        wrong_signature.install(
            signed_package(&untrusted_key, 1, &artifact, ValidatorLicense::OpenSource,),
            artifact.clone(),
        ),
        Err(ValidatorHostingError::SignatureInvalid)
    ));

    let mut tampered = trusted_host(&trusted_key);
    assert!(matches!(
        tampered.install(
            signed_package(&trusted_key, 1, &artifact, ValidatorLicense::OpenSource,),
            b"tampered artifact".to_vec(),
        ),
        Err(ValidatorHostingError::ArtifactDigestMismatch)
    ));
}

#[test]
fn m7c_revocation_and_external_paid_license_state_fail_closed() {
    let key = signing_key(21);
    let artifact = b"paid validator artifact".to_vec();
    let mut host = trusted_host(&key);
    host.install(
        signed_package(
            &key,
            1,
            &artifact,
            ValidatorLicense::Paid {
                product_id: "validator.pro.subscription".to_owned(),
            },
        ),
        artifact,
    )
    .unwrap();

    assert!(matches!(
        host.resolve(PACKAGE),
        Err(ValidatorHostingError::LicenseMissing)
    ));
    host.set_license_state(PACKAGE, LicenseState::Expired);
    assert!(matches!(
        host.resolve(PACKAGE),
        Err(ValidatorHostingError::LicenseExpired)
    ));
    host.set_license_state(PACKAGE, LicenseState::Active);
    assert!(host.resolve(PACKAGE).is_ok());

    host.revoke_release(PACKAGE, 1);
    assert!(matches!(
        host.resolve(PACKAGE),
        Err(ValidatorHostingError::ReleaseRevoked)
    ));
}

#[test]
fn m7c_binds_only_the_authenticated_descriptor_and_never_mutates_the_document() {
    let key = signing_key(25);
    let artifact = b"descriptor binding fixture".to_vec();
    let mut host = trusted_host(&key);
    host.install(
        signed_package(&key, 1, &artifact, ValidatorLicense::OpenSource),
        artifact,
    )
    .unwrap();
    let store = DocumentStore::new();
    let canonical_before = store.current().canonical_digest();

    let descriptor = descriptor(1);
    let ready = host.resolve_for_invocation(
        PACKAGE,
        &store.current(),
        invocation(&store, &descriptor),
        EvidenceClass::Exact,
    );
    assert!(matches!(ready, HostedValidatorResolution::Ready(_)));

    let mut mismatched_descriptor = descriptor;
    mismatched_descriptor.limits.maximum_work_units += 1;
    let unavailable = host.resolve_for_invocation(
        PACKAGE,
        &store.current(),
        invocation(&store, &mismatched_descriptor),
        EvidenceClass::Exact,
    );
    let HostedValidatorResolution::Unavailable(report) = unavailable else {
        panic!("descriptor mismatch must fail closed");
    };
    assert_eq!(report.state, ValidationState::Unavailable);
    assert_eq!(report.diagnostics[0].code, "validator.unavailable");
    assert!(
        report.diagnostics[0]
            .evidence
            .contains("authenticated package descriptor")
    );
    assert_eq!(store.current().canonical_digest(), canonical_before);
    assert_eq!(store.visible_undo_steps(), 0);
}

#[test]
fn m7c_stale_invocation_is_rejected_before_runtime() {
    let key = signing_key(26);
    let artifact = b"stale invocation fixture".to_vec();
    let mut host = trusted_host(&key);
    host.install(
        signed_package(&key, 1, &artifact, ValidatorLicense::OpenSource),
        artifact,
    )
    .unwrap();
    let mut store = DocumentStore::new();
    let descriptor = descriptor(1);
    let stale_invocation = invocation(&store, &descriptor);
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(501),
                name: "Revision advance".to_owned(),
            },
        ]))
        .unwrap();

    let HostedValidatorResolution::Unavailable(report) = host.resolve_for_invocation(
        PACKAGE,
        &store.current(),
        stale_invocation,
        EvidenceClass::Exact,
    ) else {
        panic!("stale invocation must fail closed");
    };
    assert_eq!(report.state, ValidationState::Unavailable);
    assert!(report.diagnostics[0].evidence.contains("stale"));
    assert_eq!(store.visible_undo_steps(), 1);
}

#[test]
fn m7c_unlicensed_package_resolves_to_structured_unavailable() {
    let key = signing_key(27);
    let artifact = b"unlicensed resolution fixture".to_vec();
    let mut host = trusted_host(&key);
    host.install(
        signed_package(
            &key,
            1,
            &artifact,
            ValidatorLicense::Paid {
                product_id: "validator.pro.subscription".to_owned(),
            },
        ),
        artifact,
    )
    .unwrap();
    let store = DocumentStore::new();
    let descriptor = descriptor(1);
    let HostedValidatorResolution::Unavailable(report) = host.resolve_for_invocation(
        PACKAGE,
        &store.current(),
        invocation(&store, &descriptor),
        EvidenceClass::Exact,
    ) else {
        panic!("missing external license state must fail closed");
    };
    assert_eq!(report.state, ValidationState::Unavailable);
    assert!(
        report.diagnostics[0]
            .evidence
            .contains("license is missing")
    );
    assert_eq!(store.visible_undo_steps(), 0);
}

#[test]
fn m7c_publisher_revocation_invalidates_an_already_installed_release() {
    let key = signing_key(31);
    let artifact = b"publisher revocation fixture".to_vec();
    let mut host = trusted_host(&key);
    host.install(
        signed_package(&key, 1, &artifact, ValidatorLicense::OpenSource),
        artifact,
    )
    .unwrap();
    assert!(host.resolve(PACKAGE).is_ok());

    host.revoke_publisher(PUBLISHER);
    assert!(matches!(
        host.resolve(PACKAGE),
        Err(ValidatorHostingError::PublisherRevoked)
    ));
}
