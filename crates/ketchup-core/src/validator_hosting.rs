use crate::document::Snapshot;
use crate::graph::sha256_hex;
use crate::validation::{
    EvidenceClass, ReadScope, ValidationClass, ValidationInvocation, ValidationReport,
    ValidatorDescriptor,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const VALIDATOR_PACKAGE_SCHEMA_V1: &str = "ketchup.validator-package.v1";
pub const MAX_VALIDATOR_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidatorRuntime {
    WasmNoImports,
    NativeSandboxed,
}

impl ValidatorRuntime {
    const fn protocol_name(self) -> &'static str {
        match self {
            Self::WasmNoImports => "wasm-no-imports.v1",
            Self::NativeSandboxed => "native-sandboxed.v1",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidatorLicense {
    OpenSource,
    Paid { product_id: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LicenseState {
    Missing,
    Active,
    Expired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorPackageManifest {
    package: String,
    version: String,
    release: u64,
    publisher: String,
    contract_id: String,
    contract_version: u32,
    implementation_id: String,
    implementation_version: String,
    descriptor_sha256: String,
    artifact_sha256: String,
    runtime: ValidatorRuntime,
    allowed_egress_hosts: Vec<String>,
    license: ValidatorLicense,
}

impl ValidatorPackageManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        package: impl Into<String>,
        version: impl Into<String>,
        release: u64,
        publisher: impl Into<String>,
        contract_id: impl Into<String>,
        contract_version: u32,
        implementation_id: impl Into<String>,
        implementation_version: impl Into<String>,
        descriptor_sha256: impl Into<String>,
        artifact_sha256: impl Into<String>,
        runtime: ValidatorRuntime,
        allowed_egress_hosts: Vec<String>,
        license: ValidatorLicense,
    ) -> Result<Self, ValidatorHostingError> {
        let mut manifest = Self {
            package: package.into(),
            version: version.into(),
            release,
            publisher: publisher.into(),
            contract_id: contract_id.into(),
            contract_version,
            implementation_id: implementation_id.into(),
            implementation_version: implementation_version.into(),
            descriptor_sha256: descriptor_sha256.into(),
            artifact_sha256: artifact_sha256.into(),
            runtime,
            allowed_egress_hosts,
            license,
        };
        manifest.allowed_egress_hosts.sort();
        manifest.allowed_egress_hosts.dedup();
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<(), ValidatorHostingError> {
        for (name, value, maximum) in [
            ("package", self.package.as_str(), 128),
            ("version", self.version.as_str(), 64),
            ("publisher", self.publisher.as_str(), 128),
            ("contract id", self.contract_id.as_str(), 160),
            ("implementation id", self.implementation_id.as_str(), 160),
            (
                "implementation version",
                self.implementation_version.as_str(),
                64,
            ),
        ] {
            if !valid_identifier(value, maximum) {
                return Err(ValidatorHostingError::InvalidManifest(format!(
                    "{name} must be a 1..{maximum} byte ASCII identifier"
                )));
            }
        }
        if self.release == 0 || self.contract_version == 0 {
            return Err(ValidatorHostingError::InvalidManifest(
                "release and contract version must be non-zero".to_owned(),
            ));
        }
        if !valid_sha256(&self.descriptor_sha256) {
            return Err(ValidatorHostingError::InvalidManifest(
                "descriptor digest must be 64 hexadecimal bytes".to_owned(),
            ));
        }
        if !valid_sha256(&self.artifact_sha256) {
            return Err(ValidatorHostingError::InvalidManifest(
                "artifact digest must be 64 hexadecimal bytes".to_owned(),
            ));
        }
        if self.allowed_egress_hosts.len() > 16
            || self
                .allowed_egress_hosts
                .iter()
                .any(|host| !valid_host(host))
        {
            return Err(ValidatorHostingError::InvalidManifest(
                "egress declaration must contain at most 16 ASCII host names".to_owned(),
            ));
        }
        if let ValidatorLicense::Paid { product_id } = &self.license
            && !valid_identifier(product_id, 128)
        {
            return Err(ValidatorHostingError::InvalidManifest(
                "paid product id must be a 1..128 byte ASCII identifier".to_owned(),
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn package(&self) -> &str {
        &self.package
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn release(&self) -> u64 {
        self.release
    }

    #[must_use]
    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    #[must_use]
    pub fn contract_id(&self) -> &str {
        &self.contract_id
    }

    #[must_use]
    pub const fn contract_version(&self) -> u32 {
        self.contract_version
    }

    #[must_use]
    pub fn implementation_id(&self) -> &str {
        &self.implementation_id
    }

    #[must_use]
    pub fn implementation_version(&self) -> &str {
        &self.implementation_version
    }

    #[must_use]
    pub fn descriptor_sha256(&self) -> &str {
        &self.descriptor_sha256
    }

    #[must_use]
    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }

    #[must_use]
    pub const fn runtime(&self) -> ValidatorRuntime {
        self.runtime
    }

    #[must_use]
    pub fn allowed_egress_hosts(&self) -> &[String] {
        &self.allowed_egress_hosts
    }

    #[must_use]
    pub const fn license(&self) -> &ValidatorLicense {
        &self.license
    }

    #[must_use]
    pub fn signing_payload(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_field(&mut bytes, VALIDATOR_PACKAGE_SCHEMA_V1.as_bytes());
        push_field(&mut bytes, self.package.as_bytes());
        push_field(&mut bytes, self.version.as_bytes());
        push_u64(&mut bytes, self.release);
        push_field(&mut bytes, self.publisher.as_bytes());
        push_field(&mut bytes, self.contract_id.as_bytes());
        push_u64(&mut bytes, u64::from(self.contract_version));
        push_field(&mut bytes, self.implementation_id.as_bytes());
        push_field(&mut bytes, self.implementation_version.as_bytes());
        push_field(&mut bytes, self.descriptor_sha256.as_bytes());
        push_field(&mut bytes, self.artifact_sha256.as_bytes());
        push_field(&mut bytes, self.runtime.protocol_name().as_bytes());
        push_u64(
            &mut bytes,
            u64::try_from(self.allowed_egress_hosts.len()).unwrap_or(u64::MAX),
        );
        for host in &self.allowed_egress_hosts {
            push_field(&mut bytes, host.as_bytes());
        }
        match &self.license {
            ValidatorLicense::OpenSource => push_field(&mut bytes, b"open-source"),
            ValidatorLicense::Paid { product_id } => {
                push_field(&mut bytes, b"paid");
                push_field(&mut bytes, product_id.as_bytes());
            }
        }
        bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SignedValidatorPackage {
    manifest: ValidatorPackageManifest,
    signature: [u8; 64],
}

impl SignedValidatorPackage {
    #[must_use]
    pub const fn new(manifest: ValidatorPackageManifest, signature: [u8; 64]) -> Self {
        Self {
            manifest,
            signature,
        }
    }

    #[must_use]
    pub const fn manifest(&self) -> &ValidatorPackageManifest {
        &self.manifest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstalledValidatorPackage {
    signed: SignedValidatorPackage,
    artifact: Vec<u8>,
}

impl InstalledValidatorPackage {
    #[must_use]
    pub const fn manifest(&self) -> &ValidatorPackageManifest {
        self.signed.manifest()
    }

    #[must_use]
    pub fn artifact(&self) -> &[u8] {
        &self.artifact
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallOutcome {
    Installed,
    Updated,
}

#[derive(Debug)]
pub enum HostedValidatorResolution<'a> {
    Ready(&'a InstalledValidatorPackage),
    Unavailable(Box<ValidationReport>),
}

#[derive(Default)]
pub struct ValidatorPackageHost {
    trusted_publishers: BTreeMap<String, VerifyingKey>,
    revoked_publishers: BTreeSet<String>,
    revoked_releases: BTreeSet<(String, u64)>,
    license_states: BTreeMap<String, LicenseState>,
    installed: BTreeMap<String, InstalledValidatorPackage>,
}

impl ValidatorPackageHost {
    pub fn trust_publisher(
        &mut self,
        publisher: impl Into<String>,
        verifying_key: [u8; 32],
    ) -> Result<(), ValidatorHostingError> {
        let publisher = publisher.into();
        if !valid_identifier(&publisher, 128) {
            return Err(ValidatorHostingError::InvalidPublisher);
        }
        let verifying_key = VerifyingKey::from_bytes(&verifying_key)
            .map_err(|_| ValidatorHostingError::InvalidPublisherKey)?;
        self.revoked_publishers.remove(&publisher);
        self.trusted_publishers.insert(publisher, verifying_key);
        Ok(())
    }

    pub fn revoke_publisher(&mut self, publisher: impl Into<String>) {
        self.revoked_publishers.insert(publisher.into());
    }

    pub fn revoke_release(&mut self, package: impl Into<String>, release: u64) {
        self.revoked_releases.insert((package.into(), release));
    }

    pub fn set_license_state(&mut self, package: impl Into<String>, state: LicenseState) {
        self.license_states.insert(package.into(), state);
    }

    pub fn install(
        &mut self,
        signed: SignedValidatorPackage,
        artifact: Vec<u8>,
    ) -> Result<InstallOutcome, ValidatorHostingError> {
        self.verify_package(&signed, &artifact)?;
        let package = signed.manifest.package.clone();
        let outcome = match self.installed.get(&package) {
            Some(current) if current.manifest().release >= signed.manifest.release => {
                return Err(ValidatorHostingError::ReleaseNotNewer {
                    installed: current.manifest().release,
                    candidate: signed.manifest.release,
                });
            }
            Some(_) => InstallOutcome::Updated,
            None => InstallOutcome::Installed,
        };
        self.installed
            .insert(package, InstalledValidatorPackage { signed, artifact });
        Ok(outcome)
    }

    #[must_use]
    pub fn discover(&self) -> Vec<&ValidatorPackageManifest> {
        self.installed
            .values()
            .map(InstalledValidatorPackage::manifest)
            .collect()
    }

    pub fn resolve(
        &self,
        package: &str,
    ) -> Result<&InstalledValidatorPackage, ValidatorHostingError> {
        let installed = self
            .installed
            .get(package)
            .ok_or_else(|| ValidatorHostingError::NotInstalled(package.to_owned()))?;
        self.verify_package(&installed.signed, &installed.artifact)?;
        if matches!(installed.manifest().license, ValidatorLicense::Paid { .. }) {
            match self
                .license_states
                .get(package)
                .copied()
                .unwrap_or(LicenseState::Missing)
            {
                LicenseState::Active => {}
                LicenseState::Missing => return Err(ValidatorHostingError::LicenseMissing),
                LicenseState::Expired => return Err(ValidatorHostingError::LicenseExpired),
            }
        }
        Ok(installed)
    }

    pub fn resolve_for_invocation(
        &self,
        package: &str,
        snapshot: &Snapshot,
        invocation: ValidationInvocation,
        evidence_class: EvidenceClass,
    ) -> HostedValidatorResolution<'_> {
        match self.bind_invocation(package, snapshot, &invocation) {
            Ok(installed) => HostedValidatorResolution::Ready(installed),
            Err(error) => HostedValidatorResolution::Unavailable(Box::new(
                ValidationReport::unavailable(invocation, evidence_class, error.to_string()),
            )),
        }
    }

    fn bind_invocation(
        &self,
        package: &str,
        snapshot: &Snapshot,
        invocation: &ValidationInvocation,
    ) -> Result<&InstalledValidatorPackage, ValidatorHostingError> {
        if !invocation.is_current(snapshot) {
            return Err(ValidatorHostingError::InvocationStale);
        }
        let installed = self.resolve(package)?;
        let manifest = installed.manifest();
        let invocation_descriptor = ValidatorDescriptor {
            contract_id: invocation.contract_id.clone(),
            contract_version: invocation.contract_version,
            implementation_id: invocation.implementation_id.clone(),
            implementation_version: invocation.implementation_version.clone(),
            input_schema: invocation.input_schema.clone(),
            validation_class: invocation.validation_class,
            read_scopes: invocation.read_scopes.clone(),
            deterministic: invocation.deterministic,
            limits: invocation.resource_limits,
        };
        if manifest.contract_id != invocation.contract_id
            || manifest.contract_version != invocation.contract_version
            || manifest.implementation_id != invocation.implementation_id
            || manifest.implementation_version != invocation.implementation_version
            || manifest.descriptor_sha256.to_ascii_lowercase()
                != validator_descriptor_digest(&invocation_descriptor)
        {
            return Err(ValidatorHostingError::DescriptorMismatch);
        }
        Ok(installed)
    }

    fn verify_package(
        &self,
        signed: &SignedValidatorPackage,
        artifact: &[u8],
    ) -> Result<(), ValidatorHostingError> {
        let manifest = signed.manifest();
        manifest.validate()?;
        if artifact.len() > MAX_VALIDATOR_ARTIFACT_BYTES {
            return Err(ValidatorHostingError::ArtifactTooLarge);
        }
        if sha256_hex(artifact) != manifest.artifact_sha256.to_ascii_lowercase() {
            return Err(ValidatorHostingError::ArtifactDigestMismatch);
        }
        if self.revoked_publishers.contains(&manifest.publisher) {
            return Err(ValidatorHostingError::PublisherRevoked);
        }
        if self
            .revoked_releases
            .contains(&(manifest.package.clone(), manifest.release))
        {
            return Err(ValidatorHostingError::ReleaseRevoked);
        }
        let key = self
            .trusted_publishers
            .get(&manifest.publisher)
            .ok_or(ValidatorHostingError::PublisherUntrusted)?;
        key.verify(
            &manifest.signing_payload(),
            &Signature::from_bytes(&signed.signature),
        )
        .map_err(|_| ValidatorHostingError::SignatureInvalid)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidatorHostingError {
    InvalidManifest(String),
    InvalidPublisher,
    InvalidPublisherKey,
    PublisherUntrusted,
    PublisherRevoked,
    ReleaseRevoked,
    SignatureInvalid,
    ArtifactTooLarge,
    ArtifactDigestMismatch,
    InvocationStale,
    DescriptorMismatch,
    ReleaseNotNewer { installed: u64, candidate: u64 },
    NotInstalled(String),
    LicenseMissing,
    LicenseExpired,
}

impl fmt::Display for ValidatorHostingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest(message) => {
                write!(formatter, "invalid validator manifest: {message}")
            }
            Self::InvalidPublisher => formatter.write_str("invalid publisher identity"),
            Self::InvalidPublisherKey => formatter.write_str("invalid Ed25519 publisher key"),
            Self::PublisherUntrusted => formatter.write_str("validator publisher is not trusted"),
            Self::PublisherRevoked => formatter.write_str("validator publisher is revoked"),
            Self::ReleaseRevoked => formatter.write_str("validator package release is revoked"),
            Self::SignatureInvalid => formatter.write_str("validator package signature is invalid"),
            Self::ArtifactTooLarge => {
                formatter.write_str("validator artifact exceeds the host byte limit")
            }
            Self::ArtifactDigestMismatch => {
                formatter.write_str("validator artifact digest does not match the signed manifest")
            }
            Self::InvocationStale => {
                formatter.write_str("validator invocation is stale for the current snapshot")
            }
            Self::DescriptorMismatch => formatter.write_str(
                "validator invocation does not match the authenticated package descriptor",
            ),
            Self::ReleaseNotNewer {
                installed,
                candidate,
            } => write!(
                formatter,
                "validator release {candidate} does not advance installed release {installed}"
            ),
            Self::NotInstalled(package) => {
                write!(formatter, "validator package {package:?} is not installed")
            }
            Self::LicenseMissing => formatter.write_str("validator license is missing"),
            Self::LicenseExpired => formatter.write_str("validator license is expired"),
        }
    }
}

impl std::error::Error for ValidatorHostingError {}

#[must_use]
pub fn validator_descriptor_digest(descriptor: &ValidatorDescriptor) -> String {
    let mut bytes = Vec::new();
    push_field(&mut bytes, descriptor.contract_id.as_bytes());
    push_u64(&mut bytes, u64::from(descriptor.contract_version));
    push_field(&mut bytes, descriptor.implementation_id.as_bytes());
    push_field(&mut bytes, descriptor.implementation_version.as_bytes());
    push_field(&mut bytes, descriptor.input_schema.as_bytes());
    push_field(
        &mut bytes,
        validation_class_name(descriptor.validation_class).as_bytes(),
    );
    let mut scopes = descriptor
        .read_scopes
        .iter()
        .map(|scope| read_scope_name(*scope))
        .collect::<Vec<_>>();
    scopes.sort_unstable();
    scopes.dedup();
    push_u64(&mut bytes, u64::try_from(scopes.len()).unwrap_or(u64::MAX));
    for scope in scopes {
        push_field(&mut bytes, scope.as_bytes());
    }
    push_field(
        &mut bytes,
        if descriptor.deterministic {
            b"deterministic"
        } else {
            b"non-deterministic"
        },
    );
    push_u64(&mut bytes, descriptor.limits.maximum_input_bytes);
    push_u64(&mut bytes, descriptor.limits.maximum_work_units);
    sha256_hex(&bytes)
}

const fn validation_class_name(class: ValidationClass) -> &'static str {
    match class {
        ValidationClass::CanonicalInvariant => "canonical-invariant",
        ValidationClass::Collision => "collision",
        ValidationClass::DeclaredJoint => "declared-joint",
        ValidationClass::StructuralBestEffort => "structural-best-effort",
        ValidationClass::Manufacturability => "manufacturability",
        ValidationClass::Advisory => "advisory",
    }
}

const fn read_scope_name(scope: ReadScope) -> &'static str {
    match scope {
        ReadScope::CanonicalGraph => "canonical-graph",
        ReadScope::DerivedGeometry => "derived-geometry",
        ReadScope::DeclaredJoints => "declared-joints",
        ReadScope::Materials => "materials",
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn valid_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':' | b'/')
        })
}

fn valid_host(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn push_field(bytes: &mut Vec<u8>, value: &[u8]) {
    push_u64(bytes, u64::try_from(value.len()).unwrap_or(u64::MAX));
    bytes.extend_from_slice(value);
}
