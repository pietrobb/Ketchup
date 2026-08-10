use ketchup_core::document::{HighRiskClass, SideEffectAuthorizationReceipt};
use ketchup_core::graph::sha256_hex;
use ketchup_core::validator_hosting::{InstalledValidatorPackage, ValidatorRuntime};
use std::collections::BTreeSet;
use std::fmt;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;
use wasmi::{Config, EnforcedLimits, Engine, Linker, Module, Store, StoreLimitsBuilder};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WasmRuntimeLimits {
    pub fuel: u64,
    pub memory_bytes: usize,
    pub table_elements: usize,
}

impl WasmRuntimeLimits {
    pub const M7C: Self = Self {
        fuel: 100_000,
        memory_bytes: 2 * 1024 * 1024,
        table_elements: 128,
    };
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WasmExecutionReceipt {
    pub package: String,
    pub release: u64,
    pub artifact_sha256: String,
    pub result_code: i32,
    pub consumed_fuel: u64,
    pub imported_capabilities: usize,
}

pub fn run_isolated_wasm_validator(
    package: &InstalledValidatorPackage,
    limits: WasmRuntimeLimits,
) -> Result<WasmExecutionReceipt, ValidatorRuntimeError> {
    if package.manifest().runtime() != ValidatorRuntime::WasmNoImports {
        return Err(ValidatorRuntimeError::NativeSandboxUnavailable);
    }
    if limits.fuel == 0 || limits.memory_bytes == 0 || limits.table_elements == 0 {
        return Err(ValidatorRuntimeError::InvalidLimits);
    }

    let mut config = Config::default();
    config
        .consume_fuel(true)
        .enforced_limits(EnforcedLimits::strict());
    let engine = Engine::new(&config);
    let module = Module::new(&engine, package.artifact())
        .map_err(|error| ValidatorRuntimeError::InvalidModule(error.to_string()))?;
    if module.imports().next().is_some() {
        return Err(ValidatorRuntimeError::ImportsDenied);
    }

    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.memory_bytes)
        .table_elements(limits.table_elements)
        .instances(1)
        .memories(1)
        .tables(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(&engine, store_limits);
    store.limiter(|state| state);
    store
        .set_fuel(limits.fuel)
        .map_err(|error| ValidatorRuntimeError::Execution(error.to_string()))?;
    let linker = Linker::<wasmi::StoreLimits>::new(&engine);
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|error| ValidatorRuntimeError::Execution(error.to_string()))?;
    let validate = instance
        .get_typed_func::<(), i32>(&store, "validate")
        .map_err(|error| ValidatorRuntimeError::Execution(error.to_string()))?;
    let result_code = validate
        .call(&mut store, ())
        .map_err(|error| ValidatorRuntimeError::Execution(error.to_string()))?;
    let remaining_fuel = store
        .get_fuel()
        .map_err(|error| ValidatorRuntimeError::Execution(error.to_string()))?;

    Ok(WasmExecutionReceipt {
        package: package.manifest().package().to_owned(),
        release: package.manifest().release(),
        artifact_sha256: sha256_hex(package.artifact()),
        result_code,
        consumed_fuel: limits.fuel.saturating_sub(remaining_fuel),
        imported_capabilities: 0,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EgressLimits {
    pub maximum_request_bytes: usize,
    pub maximum_response_bytes: usize,
    pub timeout: Duration,
}

impl EgressLimits {
    pub const M7C: Self = Self {
        maximum_request_bytes: 16 * 1024,
        maximum_response_bytes: 64 * 1024,
        timeout: Duration::from_secs(5),
    };
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EgressGrant {
    endpoints: BTreeSet<(String, u16)>,
}

impl EgressGrant {
    #[must_use]
    pub fn new(endpoints: impl IntoIterator<Item = (String, u16)>) -> Self {
        Self {
            endpoints: endpoints.into_iter().collect(),
        }
    }

    fn allows(&self, host: &str, port: u16) -> bool {
        self.endpoints.contains(&(host.to_owned(), port))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EgressRequest {
    pub host: String,
    pub port: u16,
    pub payload: Vec<u8>,
}

pub const VALIDATOR_EGRESS_OPERATION: &str = "validator-remote-egress";

#[must_use]
pub fn validator_egress_destination(request: &EgressRequest) -> String {
    format!("tcp://{}:{}", request.host, request.port)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EgressReceipt {
    pub host: String,
    pub port: u16,
    pub request_bytes: usize,
    pub response_bytes: usize,
    pub request_sha256: String,
    pub response_sha256: String,
}

pub fn perform_host_mediated_egress(
    package: &InstalledValidatorPackage,
    grant: &EgressGrant,
    request: &EgressRequest,
    authorization: Option<SideEffectAuthorizationReceipt>,
    limits: EgressLimits,
) -> Result<(Vec<u8>, EgressReceipt), ValidatorRuntimeError> {
    if !grant.allows(&request.host, request.port)
        || !package
            .manifest()
            .allowed_egress_hosts()
            .iter()
            .any(|host| host == &request.host)
    {
        return Err(ValidatorRuntimeError::EgressDenied);
    }
    if request.port == 0 || request.payload.len() > limits.maximum_request_bytes {
        return Err(ValidatorRuntimeError::EgressRequestLimitExceeded);
    }
    let authorization = authorization.ok_or(ValidatorRuntimeError::EgressAuthorizationRequired)?;
    let scope = authorization.scope();
    if authorization.operation() != VALIDATOR_EGRESS_OPERATION
        || authorization.payload_digest() != sha256_hex(&request.payload)
        || scope.class() != HighRiskClass::ExternalDisclosure
        || scope.destination() != Some(validator_egress_destination(request).as_str())
        || scope.provider() != Some(package.manifest().package())
        || scope.path().is_some()
    {
        return Err(ValidatorRuntimeError::EgressAuthorizationInvalid);
    }
    let address = (request.host.as_str(), request.port)
        .to_socket_addrs()
        .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?
        .next()
        .ok_or_else(|| {
            ValidatorRuntimeError::EgressTransport("host resolved to no address".to_owned())
        })?;
    let mut stream = TcpStream::connect_timeout(&address, limits.timeout)
        .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
    stream
        .set_read_timeout(Some(limits.timeout))
        .and_then(|()| stream.set_write_timeout(Some(limits.timeout)))
        .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
    stream
        .write_all(&request.payload)
        .and_then(|()| stream.shutdown(Shutdown::Write))
        .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;

    let response_limit = u64::try_from(limits.maximum_response_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let mut response = Vec::new();
    stream
        .take(response_limit)
        .read_to_end(&mut response)
        .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
    if response.len() > limits.maximum_response_bytes {
        return Err(ValidatorRuntimeError::EgressResponseLimitExceeded);
    }
    let receipt = EgressReceipt {
        host: request.host.clone(),
        port: request.port,
        request_bytes: request.payload.len(),
        response_bytes: response.len(),
        request_sha256: sha256_hex(&request.payload),
        response_sha256: sha256_hex(&response),
    };
    Ok((response, receipt))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidatorRuntimeError {
    NativeSandboxUnavailable,
    InvalidLimits,
    InvalidModule(String),
    ImportsDenied,
    Execution(String),
    EgressDenied,
    EgressAuthorizationRequired,
    EgressAuthorizationInvalid,
    EgressRequestLimitExceeded,
    EgressResponseLimitExceeded,
    EgressTransport(String),
}

impl fmt::Display for ValidatorRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeSandboxUnavailable => {
                formatter.write_str("native validator has no configured OS sandbox")
            }
            Self::InvalidLimits => formatter.write_str("validator runtime limits must be non-zero"),
            Self::InvalidModule(error) => {
                write!(formatter, "invalid validator Wasm module: {error}")
            }
            Self::ImportsDenied => formatter.write_str("validator Wasm imports are denied"),
            Self::Execution(error) => write!(formatter, "validator Wasm execution failed: {error}"),
            Self::EgressDenied => formatter.write_str("validator egress host is not allowlisted"),
            Self::EgressAuthorizationRequired => formatter
                .write_str("validator egress requires human external-disclosure authorization"),
            Self::EgressAuthorizationInvalid => formatter
                .write_str("validator egress authorization does not match the exact disclosure"),
            Self::EgressRequestLimitExceeded => {
                formatter.write_str("validator egress request exceeds its byte envelope")
            }
            Self::EgressResponseLimitExceeded => {
                formatter.write_str("validator egress response exceeds its byte envelope")
            }
            Self::EgressTransport(error) => write!(formatter, "validator egress failed: {error}"),
        }
    }
}

impl std::error::Error for ValidatorRuntimeError {}
