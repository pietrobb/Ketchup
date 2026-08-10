use ed25519_dalek::{Signer, SigningKey};
use ketchup_core::document::{
    AuthenticatedApprover, DocumentStore, HighRiskClass, HighRiskScope, ProposalPrincipal,
    SideEffectAuthorizationReceipt, TrustedConfirmationSurface,
};
use ketchup_core::graph::sha256_hex;
use ketchup_core::validation::{ReadScope, ResourceLimits, ValidationClass, ValidatorDescriptor};
use ketchup_core::validator_hosting::{
    InstalledValidatorPackage, SignedValidatorPackage, ValidatorLicense, ValidatorPackageHost,
    ValidatorPackageManifest, ValidatorRuntime, validator_descriptor_digest,
};
use ketchup_scheduler::validator_runtime::{
    EgressGrant, EgressLimits, EgressRequest, VALIDATOR_EGRESS_OPERATION, ValidatorRuntimeError,
    WasmRuntimeLimits, perform_host_mediated_egress, run_isolated_wasm_validator,
    validator_egress_destination,
};
use std::io::{Read, Write};
use std::net::TcpListener;

const PUBLISHER: &str = "org.ketchup.tests.publisher";
const PACKAGE: &str = "org.ketchup.tests.hosted-validator";
const VALID_WASM: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, 0x03,
    0x02, 0x01, 0x00, 0x07, 0x0c, 0x01, 0x08, b'v', b'a', b'l', b'i', b'd', b'a', b't', b'e', 0x00,
    0x00, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x07, 0x0b,
];

fn descriptor() -> ValidatorDescriptor {
    ValidatorDescriptor {
        contract_id: "ketchup.validator.prismatic-joints.v1".to_owned(),
        contract_version: 1,
        implementation_id: "org.ketchup.tests.hosted-validator.wasm.v1".to_owned(),
        implementation_version: "1.0.0".to_owned(),
        input_schema: "ketchup.prismatic-joint-input.v1".to_owned(),
        validation_class: ValidationClass::DeclaredJoint,
        read_scopes: vec![ReadScope::DerivedGeometry, ReadScope::DeclaredJoints],
        deterministic: true,
        limits: ResourceLimits {
            maximum_input_bytes: 16 * 1024,
            maximum_work_units: 100_000,
        },
    }
}

fn installed(
    artifact: &[u8],
    runtime: ValidatorRuntime,
    egress_hosts: Vec<String>,
) -> ValidatorPackageHost {
    let key = SigningKey::from_bytes(&[47; 32]);
    let descriptor = descriptor();
    let manifest = ValidatorPackageManifest::new(
        PACKAGE,
        "1.0.0",
        1,
        PUBLISHER,
        &descriptor.contract_id,
        descriptor.contract_version,
        &descriptor.implementation_id,
        &descriptor.implementation_version,
        validator_descriptor_digest(&descriptor),
        sha256_hex(artifact),
        runtime,
        egress_hosts,
        ValidatorLicense::OpenSource,
    )
    .unwrap();
    let signature = key.sign(&manifest.signing_payload()).to_bytes();
    let mut host = ValidatorPackageHost::default();
    host.trust_publisher(PUBLISHER, key.verifying_key().to_bytes())
        .unwrap();
    host.install(
        SignedValidatorPackage::new(manifest, signature),
        artifact.to_vec(),
    )
    .unwrap();
    host
}

fn authorize_external_disclosure(
    package: &InstalledValidatorPackage,
    request: &EgressRequest,
) -> SideEffectAuthorizationReceipt {
    authorize_external_disclosure_for(
        request,
        validator_egress_destination(request),
        package.manifest().package().to_owned(),
    )
}

fn authorize_external_disclosure_for(
    request: &EgressRequest,
    destination: String,
    provider: String,
) -> SideEffectAuthorizationReceipt {
    let authority = TrustedConfirmationSurface::new([53; 32], 4).unwrap();
    let mut document = DocumentStore::new();
    document
        .configure_human_confirmation_policy(authority.verifying_key(), 4)
        .unwrap();
    let scope = HighRiskScope::new(
        HighRiskClass::ExternalDisclosure,
        Some(destination),
        Some(provider),
        None,
    )
    .unwrap();
    let proposal = document
        .prepare_high_risk_side_effect(
            VALIDATOR_EGRESS_OPERATION,
            ProposalPrincipal::Plugin(71),
            scope,
            &request.payload,
        )
        .unwrap();
    let approval = authority
        .issue_side_effect(&proposal, AuthenticatedApprover::Human(72), 10_000, 11_000)
        .unwrap();
    document
        .authorize_high_risk_side_effect(&proposal, &approval, 10_500)
        .unwrap()
}

#[test]
fn m7c_wasm_validator_has_no_ambient_imports_and_is_fuel_and_memory_bounded() {
    let host = installed(VALID_WASM, ValidatorRuntime::WasmNoImports, vec![]);
    let receipt =
        run_isolated_wasm_validator(host.resolve(PACKAGE).unwrap(), WasmRuntimeLimits::M7C)
            .unwrap();
    assert_eq!(receipt.result_code, 7);
    assert_eq!(receipt.imported_capabilities, 0);
    assert!(receipt.consumed_fuel > 0);
    assert_eq!(receipt.artifact_sha256, sha256_hex(VALID_WASM));
}

#[test]
fn m7c_wasm_imports_and_unsandboxed_native_runtime_fail_closed() {
    let importing_wasm = br#"(module
        (import "env" "ambient" (func))
        (func (export "validate") (result i32) i32.const 0))"#;
    let import_host = installed(importing_wasm, ValidatorRuntime::WasmNoImports, vec![]);
    assert!(matches!(
        run_isolated_wasm_validator(
            import_host.resolve(PACKAGE).unwrap(),
            WasmRuntimeLimits::M7C,
        ),
        Err(ValidatorRuntimeError::ImportsDenied)
    ));

    let native_host = installed(
        b"native validator fixture",
        ValidatorRuntime::NativeSandboxed,
        vec![],
    );
    assert!(matches!(
        run_isolated_wasm_validator(
            native_host.resolve(PACKAGE).unwrap(),
            WasmRuntimeLimits::M7C,
        ),
        Err(ValidatorRuntimeError::NativeSandboxUnavailable)
    ));
}

#[test]
fn m7c_wasm_fuel_exhaustion_fails_closed() {
    let looping_wasm = br#"(module
        (func (export "validate") (result i32)
            (loop br 0)
            i32.const 0))"#;
    let host = installed(looping_wasm, ValidatorRuntime::WasmNoImports, vec![]);
    let limits = WasmRuntimeLimits {
        fuel: 100,
        ..WasmRuntimeLimits::M7C
    };
    assert!(matches!(
        run_isolated_wasm_validator(host.resolve(PACKAGE).unwrap(), limits),
        Err(ValidatorRuntimeError::Execution(_))
    ));
}

#[test]
fn m7c_remote_egress_is_host_mediated_allowlisted_bounded_and_receipted() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        assert_eq!(request, b"VALIDATE");
        stream.write_all(b"REMOTE-OK").unwrap();
    });
    let host = installed(
        VALID_WASM,
        ValidatorRuntime::WasmNoImports,
        vec!["127.0.0.1".to_owned()],
    );
    let package = host.resolve(PACKAGE).unwrap();
    let request = EgressRequest {
        host: "127.0.0.1".to_owned(),
        port,
        payload: b"VALIDATE".to_vec(),
    };
    assert!(matches!(
        perform_host_mediated_egress(
            package,
            &EgressGrant::default(),
            &request,
            None,
            EgressLimits::M7C
        ),
        Err(ValidatorRuntimeError::EgressDenied)
    ));
    let grant = EgressGrant::new([("127.0.0.1".to_owned(), port)]);
    let authorization = authorize_external_disclosure(package, &request);
    let (response, egress_receipt) = perform_host_mediated_egress(
        package,
        &grant,
        &request,
        Some(authorization),
        EgressLimits::M7C,
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(response, b"REMOTE-OK");
    assert_eq!(egress_receipt.request_sha256, sha256_hex(b"VALIDATE"));
    assert_eq!(egress_receipt.response_sha256, sha256_hex(b"REMOTE-OK"));
    assert_eq!(egress_receipt.response_bytes, 9);

    let denied = EgressRequest {
        host: "not-allowlisted.example".to_owned(),
        port,
        payload: vec![],
    };
    assert!(matches!(
        perform_host_mediated_egress(package, &grant, &denied, None, EgressLimits::M7C),
        Err(ValidatorRuntimeError::EgressDenied)
    ));
    let oversized = EgressRequest {
        host: "127.0.0.1".to_owned(),
        port,
        payload: vec![0; EgressLimits::M7C.maximum_request_bytes + 1],
    };
    assert!(matches!(
        perform_host_mediated_egress(package, &grant, &oversized, None, EgressLimits::M7C),
        Err(ValidatorRuntimeError::EgressRequestLimitExceeded)
    ));
}

#[test]
fn m18b_external_disclosure_requires_exact_human_authorization_before_connect() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let host = installed(
        VALID_WASM,
        ValidatorRuntime::WasmNoImports,
        vec!["127.0.0.1".to_owned()],
    );
    let package = host.resolve(PACKAGE).unwrap();
    let approved_request = EgressRequest {
        host: "127.0.0.1".to_owned(),
        port,
        payload: b"APPROVED DISCLOSURE".to_vec(),
    };
    let grant = EgressGrant::new([("127.0.0.1".to_owned(), port)]);

    assert!(matches!(
        perform_host_mediated_egress(package, &grant, &approved_request, None, EgressLimits::M7C,),
        Err(ValidatorRuntimeError::EgressAuthorizationRequired)
    ));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    let authorization = authorize_external_disclosure(package, &approved_request);
    let substituted_request = EgressRequest {
        host: approved_request.host.clone(),
        port: approved_request.port,
        payload: b"SUBSTITUTED DISCLOSURE".to_vec(),
    };
    assert!(matches!(
        perform_host_mediated_egress(
            package,
            &grant,
            &substituted_request,
            Some(authorization),
            EgressLimits::M7C,
        ),
        Err(ValidatorRuntimeError::EgressAuthorizationInvalid)
    ));

    let wrong_destination = authorize_external_disclosure_for(
        &approved_request,
        "tcp://127.0.0.1:1".to_owned(),
        package.manifest().package().to_owned(),
    );
    assert!(matches!(
        perform_host_mediated_egress(
            package,
            &grant,
            &approved_request,
            Some(wrong_destination),
            EgressLimits::M7C,
        ),
        Err(ValidatorRuntimeError::EgressAuthorizationInvalid)
    ));

    let wrong_provider = authorize_external_disclosure_for(
        &approved_request,
        validator_egress_destination(&approved_request),
        "org.ketchup.tests.other-provider".to_owned(),
    );
    assert!(matches!(
        perform_host_mediated_egress(
            package,
            &grant,
            &approved_request,
            Some(wrong_provider),
            EgressLimits::M7C,
        ),
        Err(ValidatorRuntimeError::EgressAuthorizationInvalid)
    ));
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn m7c_remote_response_overflow_fails_closed() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        let _ = stream.write_all(b"response beyond envelope");
    });
    let host = installed(
        VALID_WASM,
        ValidatorRuntime::WasmNoImports,
        vec!["127.0.0.1".to_owned()],
    );
    let request = EgressRequest {
        host: "127.0.0.1".to_owned(),
        port,
        payload: b"request".to_vec(),
    };
    let limits = EgressLimits {
        maximum_response_bytes: 4,
        ..EgressLimits::M7C
    };
    let grant = EgressGrant::new([("127.0.0.1".to_owned(), port)]);
    let package = host.resolve(PACKAGE).unwrap();
    let authorization = authorize_external_disclosure(package, &request);
    assert!(matches!(
        perform_host_mediated_egress(package, &grant, &request, Some(authorization), limits,),
        Err(ValidatorRuntimeError::EgressResponseLimitExceeded)
    ));
    server.join().unwrap();
}
