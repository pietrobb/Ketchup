//! Gives the Windows executable its icon (`assets/ketchup.ico`).
//!
//! Writes the icon as a compiled resource file (`.res`) and hands it to the
//! MSVC linker, so no resource compiler from the Windows SDK is needed.
use std::{env, fs, io, path::PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const LANG_EN_US: u16 = 0x0409;
const ICON_FLAGS: u16 = 0x1010;
const GROUP_FLAGS: u16 = 0x1030;

fn main() -> io::Result<()> {
    let icon = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("assets")
        .join("ketchup.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let windows = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|target_env| target_env == "msvc");
    if !(windows && msvc) {
        return Ok(());
    }
    let res = PathBuf::from(env::var_os("OUT_DIR").unwrap_or_default()).join("ketchup-icon.res");
    fs::write(&res, icon_resources(&fs::read(&icon)?)?)?;
    println!("cargo:rustc-link-arg-bins={}", res.display());
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("assets/ketchup.ico: {message}"),
    )
}

fn u16_at(bytes: &[u8], at: usize) -> io::Result<u16> {
    bytes
        .get(at..at + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or_else(|| invalid("truncated"))
}

fn u32_at(bytes: &[u8], at: usize) -> io::Result<u32> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| invalid("truncated"))
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
fn icon_resources(ico: &[u8]) -> io::Result<Vec<u8>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return Err(invalid("not an icon file"));
    }
    let count = u16_at(ico, 4)?;
    if count == 0 {
        return Err(invalid("no images"));
    }
    let mut out = Vec::new();
    push_resource(&mut out, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0_u16.to_le_bytes());
    group.extend_from_slice(&1_u16.to_le_bytes());
    group.extend_from_slice(&count.to_le_bytes());
    for index in 0..count {
        let entry = 6 + 16 * usize::from(index);
        let header = ico
            .get(entry..entry + 8)
            .ok_or_else(|| invalid("truncated"))?;
        let length = u32_at(ico, entry + 8)?;
        let offset = usize::try_from(u32_at(ico, entry + 12)?).map_err(|_| invalid("offset"))?;
        let length_usize = usize::try_from(length).map_err(|_| invalid("length"))?;
        let image = ico
            .get(offset..offset + length_usize)
            .ok_or_else(|| invalid("image outside the file"))?;
        let id = index + 1;
        push_resource(&mut out, RT_ICON, id, ICON_FLAGS, image);
        group.extend_from_slice(header);
        group.extend_from_slice(&length.to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut out, RT_GROUP_ICON, 1, GROUP_FLAGS, &group);
    Ok(out)
}
