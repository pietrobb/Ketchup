//! Gives a Windows executable its icon from an `.ico` in the crate's `assets`.
//!
//! Writes the icon as a compiled resource file (`.res`) and hands it to the
//! MSVC linker, so no resource compiler from the Windows SDK is needed.
//! Shared by the build scripts of `ketchup-viewer` and `ketchup-app`.
use std::{env, fs, io, path::PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
const LANG_EN_US: u16 = 0x0409;
const ICON_FLAGS: u16 = 0x1010;
const GROUP_FLAGS: u16 = 0x1030;
const VERSION_FLAGS: u16 = 0x0030;

/// Link `assets/<file_name>` into every binary of the crate being built, with
/// a version resource naming `product` and the release (feedback 2026-11-10:
/// the executable had no FileVersion or ProductVersion).
pub fn embed(file_name: &str, product: &str) -> io::Result<()> {
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("assets")
        .join(file_name);
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-env-changed=KETCHUP_BUILD_VERSION");
    // The release the programs report (`ketchup_mcp::VERSION`).
    let version = env::var("KETCHUP_BUILD_VERSION")
        .or_else(|_| env::var("CARGO_PKG_VERSION"))
        .unwrap_or_default();
    let windows = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|target_env| target_env == "msvc");
    if !(windows && msvc) {
        return Ok(());
    }
    let res = PathBuf::from(env::var_os("OUT_DIR").unwrap_or_default()).join("icon.res");
    let bytes = fs::read(&icon)?;
    let mut resources = icon_resources(&bytes).map_err(|message| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("assets/{file_name}: {message}"),
        )
    })?;
    let numeric = numeric_version(&version).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("version {version} is not major.minor.patch[-pre]"),
        )
    })?;
    push_resource(
        &mut resources,
        RT_VERSION,
        1,
        VERSION_FLAGS,
        &version_info(product, &version, numeric),
    );
    fs::write(&res, resources)?;
    println!("cargo:rustc-link-arg-bins={}", res.display());
    Ok(())
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, &'static str> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or("truncated")
}

fn u32_at(bytes: &[u8], at: usize) -> Result<u32, &'static str> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or("truncated")
}

/// One resource entry: header with ordinal type and name, data, padding to 4 bytes.
fn push_resource(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&32_u32.to_le_bytes());
    for ordinal in [kind, name] {
        out.extend_from_slice(&0xFFFF_u16.to_le_bytes());
        out.extend_from_slice(&ordinal.to_le_bytes());
    }
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&LANG_EN_US.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    out.extend_from_slice(data);
    out.resize(out.len().next_multiple_of(4), 0);
}

/// Every image of the `.ico` as an RT_ICON, plus the RT_GROUP_ICON that lists them.
fn icon_resources(ico: &[u8]) -> Result<Vec<u8>, &'static str> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return Err("not an icon file");
    }
    let count = u16_at(ico, 4)?;
    if count == 0 {
        return Err("no images");
    }
    let mut out = Vec::new();
    push_resource(&mut out, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0_u16.to_le_bytes());
    group.extend_from_slice(&1_u16.to_le_bytes());
    group.extend_from_slice(&count.to_le_bytes());
    for index in 0..count {
        let entry = 6 + 16 * usize::from(index);
        let header = ico.get(entry..entry + 8).ok_or("truncated")?;
        let length = u32_at(ico, entry + 8)?;
        let offset = usize::try_from(u32_at(ico, entry + 12)?).or(Err("offset"))?;
        let length_usize = usize::try_from(length).or(Err("length"))?;
        let image = ico
            .get(offset..offset + length_usize)
            .ok_or("image outside the file")?;
        let id = index + 1;
        push_resource(&mut out, RT_ICON, id, ICON_FLAGS, image);
        group.extend_from_slice(header);
        group.extend_from_slice(&length.to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut out, RT_GROUP_ICON, 1, GROUP_FLAGS, &group);
    Ok(out)
}

/// `major.minor.patch.0` of a release such as `0.3.3-dev`, as Windows keeps a
/// file version: four numbers of 16 bits.
fn numeric_version(version: &str) -> Option<[u16; 4]> {
    let release = version.split(['-', '+']).next()?;
    let mut parts = release.split('.').map(|part| part.parse::<u16>().ok());
    let numeric = [parts.next()??, parts.next()??, parts.next()??, 0];
    parts.next().is_none().then_some(numeric)
}

/// One node of a version resource: its length, the length of its value, its
/// kind (1 text, 0 binary), its key, then its value and children, each
/// starting on 4 bytes.
fn version_block(
    key: &str,
    value: &[u8],
    value_length: u16,
    text: bool,
    children: &[Vec<u8>],
) -> Vec<u8> {
    let mut out = vec![0, 0];
    out.extend_from_slice(&value_length.to_le_bytes());
    out.extend_from_slice(&u16::from(text).to_le_bytes());
    for unit in key.encode_utf16().chain([0]) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.resize(out.len().next_multiple_of(4), 0);
    out.extend_from_slice(value);
    for child in children {
        out.resize(out.len().next_multiple_of(4), 0);
        out.extend_from_slice(child);
    }
    let length = u16::try_from(out.len()).unwrap_or(u16::MAX);
    out[..2].copy_from_slice(&length.to_le_bytes());
    out
}

fn version_string(key: &str, value: &str) -> Vec<u8> {
    let units: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let bytes: Vec<u8> = units.iter().flat_map(|unit| unit.to_le_bytes()).collect();
    let length = u16::try_from(units.len()).unwrap_or(u16::MAX);
    version_block(key, &bytes, length, true, &[])
}

/// The VS_VERSIONINFO of a program: the numeric file and product version, and
/// in US English the product's name and the release as written.
fn version_info(product: &str, version: &str, numeric: [u16; 4]) -> Vec<u8> {
    let high = |a: u16, b: u16| (u32::from(a) << 16) | u32::from(b);
    let mut fixed = Vec::new();
    for value in [
        0xFEEF_04BD, // signature
        0x0001_0000, // structure version
        high(numeric[0], numeric[1]),
        high(numeric[2], numeric[3]),
        high(numeric[0], numeric[1]),
        high(numeric[2], numeric[3]),
        0x3F,        // flags mask
        0,           // flags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,
        0,
        0,
    ] {
        fixed.extend_from_slice(&u32::to_le_bytes(value));
    }
    let file_version = numeric.map(|part| part.to_string()).join(".");
    let strings = version_block(
        "040904B0",
        &[],
        0,
        true,
        &[
            version_string("FileDescription", product),
            version_string("FileVersion", &file_version),
            version_string("ProductName", product),
            version_string("ProductVersion", version),
        ],
    );
    let string_info = version_block("StringFileInfo", &[], 0, true, &[strings]);
    let translation = version_block("Translation", &0x04B0_0409_u32.to_le_bytes(), 4, false, &[]);
    let var_info = version_block("VarFileInfo", &[], 0, true, &[translation]);
    let fixed_length = u16::try_from(fixed.len()).unwrap_or(u16::MAX);
    version_block(
        "VS_VERSION_INFO",
        &fixed,
        fixed_length,
        false,
        &[string_info, var_info],
    )
}
