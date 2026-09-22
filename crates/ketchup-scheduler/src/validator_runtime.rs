use ketchup_core::document::{HighRiskClass, SideEffectAuthorizationReceipt};
use ketchup_core::graph::sha256_hex;
use ketchup_core::validator_hosting::{InstalledValidatorPackage, ValidatorRuntime};
use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
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
    endpoints: BTreeMap<(String, u16), SocketAddr>,
}

impl EgressGrant {
    pub fn new(
        endpoints: impl IntoIterator<Item = (String, u16)>,
    ) -> Result<Self, ValidatorRuntimeError> {
        let mut verified = BTreeMap::new();
        for (host, port) in endpoints {
            let addresses = (host.as_str(), port)
                .to_socket_addrs()
                .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?
                .collect::<Vec<_>>();
            let address = *addresses.first().ok_or_else(|| {
                ValidatorRuntimeError::EgressTransport("host resolved to no address".to_owned())
            })?;
            if host.parse::<IpAddr>().is_err()
                && addresses
                    .iter()
                    .any(|address| !is_public_egress_address(address.ip()))
            {
                return Err(ValidatorRuntimeError::EgressDenied);
            }
            verified.insert((host, port), address);
        }
        Ok(Self {
            endpoints: verified,
        })
    }

    fn address(&self, host: &str, port: u16) -> Option<SocketAddr> {
        self.endpoints.get(&(host.to_owned(), port)).copied()
    }
}

fn is_public_egress_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let octets = address.octets();
            let shared = octets[0] == 100 && octets[1] & 0xc0 == 64;
            let benchmarking = octets[0] == 198 && octets[1] & 0xfe == 18;
            let protocol_assignment = octets[0] == 192 && octets[1] == 0 && octets[2] == 0;
            !address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_broadcast()
                && !address.is_documentation()
                && octets[0] != 0
                && !shared
                && !benchmarking
                && !protocol_assignment
                && octets[0] < 240
        }
        IpAddr::V6(address) => {
            if let Some(address) = address.to_ipv4() {
                return is_public_egress_address(IpAddr::V4(address));
            }
            let first = address.segments()[0];
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && first & 0xfe00 != 0xfc00
                && first & 0xffc0 != 0xfe80
                && first & 0xffc0 != 0xfec0
                && address.segments()[..2] != [0x2001, 0x0db8]
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EgressRequest {
    pub host: String,
    pub port: u16,
    pub payload: Vec<u8>,
}

pub const VALIDATOR_EGRESS_OPERATION: &str = "validator-remote-egress";

pub fn validator_egress_destination(
    grant: &EgressGrant,
    request: &EgressRequest,
) -> Result<String, ValidatorRuntimeError> {
    let address = grant
        .address(&request.host, request.port)
        .ok_or(ValidatorRuntimeError::EgressDenied)?;
    Ok(format!(
        "tcp://{}:{}#resolved={address}",
        request.host, request.port
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EgressReceipt {
    pub host: String,
    pub port: u16,
    pub address: SocketAddr,
    pub request_bytes: usize,
    pub response_bytes: usize,
    pub request_sha256: String,
    pub response_sha256: String,
}

fn remaining_egress_time(deadline: Instant) -> Result<Duration, ValidatorRuntimeError> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| ValidatorRuntimeError::EgressTransport("operation timed out".to_owned()))
}

pub fn perform_host_mediated_egress(
    package: &InstalledValidatorPackage,
    grant: &EgressGrant,
    request: &EgressRequest,
    authorization: Option<SideEffectAuthorizationReceipt>,
    limits: EgressLimits,
) -> Result<(Vec<u8>, EgressReceipt), ValidatorRuntimeError> {
    if limits.maximum_request_bytes == 0
        || limits.maximum_response_bytes == 0
        || limits.timeout.is_zero()
    {
        return Err(ValidatorRuntimeError::InvalidLimits);
    }
    let deadline = Instant::now()
        .checked_add(limits.timeout)
        .ok_or(ValidatorRuntimeError::InvalidLimits)?;
    let address = grant
        .address(&request.host, request.port)
        .ok_or(ValidatorRuntimeError::EgressDenied)?;
    if !package
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
        || scope.destination() != Some(validator_egress_destination(grant, request)?.as_str())
        || scope.provider() != Some(package.manifest().package())
        || scope.path().is_some()
    {
        return Err(ValidatorRuntimeError::EgressAuthorizationInvalid);
    }
    let mut stream = TcpStream::connect_timeout(&address, remaining_egress_time(deadline)?)
        .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
    let mut written = 0;
    while written < request.payload.len() {
        stream
            .set_write_timeout(Some(remaining_egress_time(deadline)?))
            .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
        let count = stream
            .write(&request.payload[written..])
            .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
        if count == 0 {
            return Err(ValidatorRuntimeError::EgressTransport(
                "connection closed while writing request".to_owned(),
            ));
        }
        written += count;
    }
    remaining_egress_time(deadline)?;
    stream
        .shutdown(Shutdown::Write)
        .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;

    let response_limit = limits.maximum_response_bytes.saturating_add(1);
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8 * 1024];
    while response.len() < response_limit {
        stream
            .set_read_timeout(Some(remaining_egress_time(deadline)?))
            .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
        let available = response_limit - response.len();
        let read_bytes = available.min(buffer.len());
        let count = stream
            .read(&mut buffer[..read_bytes])
            .map_err(|error| ValidatorRuntimeError::EgressTransport(error.to_string()))?;
        if count == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..count]);
    }
    if response.len() > limits.maximum_response_bytes {
        return Err(ValidatorRuntimeError::EgressResponseLimitExceeded);
    }
    let receipt = EgressReceipt {
        host: request.host.clone(),
        port: request.port,
        address,
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
