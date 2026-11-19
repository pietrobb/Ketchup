//! The "send to workshop" package (review 2026-11-08 §5.2): the shop files of a
//! model in one stored ZIP with a report of what could not be made and a
//! SHA-256 list, and the acceptance check the workshop runs on what it got.

use crate::fabrication::{
    BtlxExportOptions, GeneralFabricationError, GeneralFabricationProjection, WoodwopMprOptions,
};
use ketchup_model::document::Snapshot;
use ketchup_model::exact_product::ExactProductError;
use ketchup_model::graph::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};
use std::str::Utf8Error;
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;

// not a tolerance: the length of a SHA-256 written in hex.
const SHA256_HEX_LEN: usize = 64;

/// The SHA-256 list, in the format `sha256sum -c` reads.
pub const WORKSHOP_SUMS: &str = "SHA256SUMS";
/// What the package could not hold, and why.
pub const WORKSHOP_REPORT: &str = "not_made.txt";

/// A shop output the package does not hold, and why.
#[derive(Clone, Debug)]
pub struct NotMade {
    pub output: String,
    pub reason: Arc<dyn std::error::Error + Send + Sync>,
}

/// The files to send, by their path in the package, and the outputs that
/// could not be made.
#[derive(Clone, Debug, Default)]
pub struct WorkshopPackage {
    pub files: Vec<(String, Vec<u8>)>,
    pub not_made: Vec<NotMade>,
}

/// What the workshop received: every file the list names, checked, and the
/// report of what was not made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopAcceptance {
    pub files: Vec<String>,
    pub not_made: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkshopPackageError {
    /// There is no file to send.
    Empty,
    /// The path is not a plain relative path inside the package.
    BadName(String),
    DuplicateName(String),
    TooLarge(ExactProductError),
    /// The archive is not a readable stored ZIP.
    NotAPackage,
    /// An entry name in the archive is not UTF-8.
    NameNotText(Utf8Error),
    NoSums,
    SumsNotText(Utf8Error),
    /// The SHA-256 list has a line that is not `<hash>  <path>`.
    BadSums(String),
    /// The list names a file the package does not have.
    Missing(String),
    /// A file differs from its hash in the list.
    Changed(String),
    /// The package has a file the list does not name.
    Unlisted(String),
    /// The local header of the entry says something else than the central
    /// directory; an unpacker reading the local headers gets another file.
    LocalHeaderDiffers(String),
    /// The archive holds bytes no central directory entry accounts for.
    HiddenData,
    /// A name that is not ASCII without the ZIP UTF-8 flag (bit 11): an
    /// unpacker reads it in its own code page and writes another name.
    NameNotMarkedUtf8(String),
}

impl std::fmt::Display for WorkshopPackageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("there is no shop file to send"),
            Self::BadName(name) => write!(formatter, "`{name}` is not a path inside the package"),
            Self::DuplicateName(name) => write!(formatter, "the package has `{name}` twice"),
            Self::TooLarge(error) => write!(formatter, "the package cannot be one ZIP: {error}"),
            Self::NotAPackage => formatter.write_str("the file is not a readable workshop package"),
            Self::NameNotText(error) => write!(formatter, "a file name is not text: {error}"),
            Self::NoSums => write!(formatter, "the package has no {WORKSHOP_SUMS}"),
            Self::SumsNotText(error) => write!(formatter, "{WORKSHOP_SUMS} is not text: {error}"),
            Self::BadSums(line) => write!(formatter, "{WORKSHOP_SUMS} cannot read `{line}`"),
            Self::Missing(name) => write!(formatter, "`{name}` is missing from the package"),
            Self::Changed(name) => write!(formatter, "`{name}` is not the file that was sent"),
            Self::Unlisted(name) => write!(formatter, "`{name}` was not sent with the package"),
            Self::LocalHeaderDiffers(name) => write!(
                formatter,
                "the stored header of `{name}` differs from the file list of the package"
            ),
            Self::HiddenData => {
                formatter.write_str("the package holds data its file list does not name")
            }
            Self::NameNotMarkedUtf8(name) => write!(
                formatter,
                "`{name}` is not marked as UTF-8 and unpacks under another name"
            ),
        }
    }
}

impl std::error::Error for WorkshopPackageError {}

impl WorkshopPackage {
    /// The package as a stored ZIP: the files, [`WORKSHOP_REPORT`] and, last,
    /// [`WORKSHOP_SUMS`] with the hash of each.
    pub fn zip(&self) -> Result<Vec<u8>, WorkshopPackageError> {
        if self.files.is_empty() {
            return Err(WorkshopPackageError::Empty);
        }
        let mut names = BTreeSet::from([folded(WORKSHOP_SUMS), folded(WORKSHOP_REPORT)]);
        for (name, _) in &self.files {
            if !is_inside(name) || name == WORKSHOP_SUMS || name == WORKSHOP_REPORT {
                return Err(WorkshopPackageError::BadName(name.clone()));
            }
            if !names.insert(folded(name)) {
                return Err(WorkshopPackageError::DuplicateName(name.clone()));
            }
        }
        let report = report_text(&self.not_made);
        let mut entries = self
            .files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        entries.push((WORKSHOP_REPORT, report.as_bytes()));
        let sums = entries
            .iter()
            .map(|(name, bytes)| format!("{}  {name}\n", sha256_hex(bytes)))
            .collect::<String>();
        entries.push((WORKSHOP_SUMS, sums.as_bytes()));
        crate::three_mf_export::encode_package(&entries, usize::MAX)
            .map_err(WorkshopPackageError::TooLarge)
    }
}

/// How the machine files of the package are written.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WorkshopPackageOptions {
    pub btlx: BtlxExportOptions,
    pub woodwop: WoodwopMprOptions,
}

impl GeneralFabricationProjection {
    /// Every shop output of the model that can be made: the parts list, the
    /// part drawings, BTLx and the woodWOP program and label of each piece.
    /// An output that cannot be made is named with its reason instead, and
    /// does not stop the others.
    pub fn workshop_package(
        &self,
        snapshot: &Snapshot,
        options: WorkshopPackageOptions,
    ) -> WorkshopPackage {
        let mut package = WorkshopPackage::default();
        let mut add =
            |output: &str, made: Result<Vec<(String, Vec<u8>)>, GeneralFabricationError>| match made
            {
                Ok(files) => package.files.extend(files),
                Err(error) => package.not_made.push(NotMade {
                    output: output.to_owned(),
                    reason: Arc::new(error),
                }),
            };
        let one = |name: &str, bytes: Vec<u8>| vec![(name.to_owned(), bytes)];
        add(
            "parts list",
            self.bom_export(snapshot)
                .map(|bytes| one("parts.csv", bytes)),
        );
        add(
            "drawings",
            self.drawing_svg(snapshot)
                .map(|bytes| one("drawings.svg", bytes)),
        );
        add(
            "BTLx",
            self.btlx_2_3_1_export_with_options(snapshot, options.btlx)
                .map(|bytes| one("btlx/model.btlx", bytes)),
        );
        // A part the machine cannot take is named by itself; the other parts
        // keep their programs.
        let woodwop = self
            .woodwop_mpr_4_0_partial_production_package(snapshot, options.woodwop)
            .and_then(|partial| {
                if partial.programs.is_empty() && partial.not_programmed.is_empty() {
                    return Err(GeneralFabricationError::NoPieceToProgram);
                }
                Ok(partial)
            });
        match woodwop {
            Ok(partial) => {
                for program in partial.programs {
                    let name = &program.program_name;
                    package.files.extend([
                        (format!("woodwop/{name}.mpr"), program.mpr),
                        (format!("woodwop/{name}.svg"), program.barcode_svg),
                    ]);
                }
                package
                    .not_made
                    .extend(partial.not_programmed.into_iter().map(|part| NotMade {
                        output: format!("woodWOP {}", part.part),
                        reason: Arc::new(part.reason),
                    }));
            }
            Err(error) => add("woodWOP", Err(error)),
        }
        package
    }
}

fn report_text(not_made: &[NotMade]) -> String {
    if not_made.is_empty() {
        return "Every output was made.\n".to_owned();
    }
    let one_line = |text: &str| text.replace(['\r', '\n'], " ");
    std::iter::once("Not made:\n".to_owned())
        .chain(not_made.iter().map(|item| {
            format!(
                "- {}: {}\n",
                one_line(&item.output),
                one_line(&item.reason.to_string())
            )
        }))
        .collect()
}

/// A plain relative path that stays inside the folder the package is
/// unpacked into and is written under this name on every system.
fn is_inside(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(char::is_control) && name.split('/').all(is_portable)
}

/// A file or folder name Windows writes as it is: not `.` or `..`, none of
/// the characters it refuses, no trailing dot or space, not a device.
fn is_portable(part: &str) -> bool {
    !part.is_empty()
        && !part.contains(['\\', ':', '<', '>', '"', '|', '?', '*'])
        && !part.ends_with(['.', ' '])
        && !is_device_name(part)
}

/// A name Windows keeps for a device, with any extension (`CON.dxf`).
fn is_device_name(part: &str) -> bool {
    let stem = part
        .split_once('.')
        .map_or(part, |(stem, _extension)| stem)
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|port| {
        stem.strip_prefix(port).is_some_and(|number| {
            matches!(number.as_bytes(), [digit] if digit.is_ascii_digit())
                || matches!(number, "\u{b9}" | "\u{b2}" | "\u{b3}")
        })
    })
}

/// The name as Windows and macOS compare names: one Unicode form, any case.
fn folded(name: &str) -> String {
    name.nfc().flat_map(char::to_lowercase).collect()
}

/// Checks a received package: every file its SHA-256 list names is there and
/// unchanged, and it has no other file.
pub fn check_workshop_package(zip: &[u8]) -> Result<WorkshopAcceptance, WorkshopPackageError> {
    let mut entries = stored_entries(zip)?;
    let sums = entries
        .remove(WORKSHOP_SUMS)
        .ok_or(WorkshopPackageError::NoSums)?;
    let sums = std::str::from_utf8(&sums).map_err(WorkshopPackageError::SumsNotText)?;
    let mut files = Vec::new();
    let mut not_made = None;
    for line in sums.lines() {
        let bad = || WorkshopPackageError::BadSums(line.to_owned());
        let (hash, name) = line
            .split_once("  ")
            .or_else(|| line.split_once(" *"))
            .ok_or_else(bad)?;
        if hash.len() != SHA256_HEX_LEN || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(bad());
        }
        if !is_inside(name) || name == WORKSHOP_SUMS || files.iter().any(|file| file == name) {
            return Err(bad());
        }
        let bytes = entries
            .remove(name)
            .ok_or_else(|| WorkshopPackageError::Missing(name.to_owned()))?;
        if !sha256_hex(&bytes).eq_ignore_ascii_case(hash) {
            return Err(WorkshopPackageError::Changed(name.to_owned()));
        }
        if name == WORKSHOP_REPORT {
            not_made = Some(String::from_utf8_lossy(&bytes).into_owned());
        }
        files.push(name.to_owned());
    }
    if let Some(name) = entries.into_keys().next() {
        return Err(WorkshopPackageError::Unlisted(name));
    }
    Ok(WorkshopAcceptance {
        files,
        not_made: not_made.ok_or_else(|| WorkshopPackageError::Missing(WORKSHOP_REPORT.into()))?,
    })
}

/// The entries of a stored ZIP, each read from where the central directory
/// points and checked against its CRC. The local header of each must say
/// what the central directory says, and every byte of the archive must
/// belong to a listed entry, the central directory or the end record, so an
/// unpacker that reads the archive another way gets the same files.
fn stored_entries(zip: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, WorkshopPackageError> {
    const LOCAL: [u8; 4] = 0x0403_4b50_u32.to_le_bytes();
    const CENTRAL: [u8; 4] = 0x0201_4b50_u32.to_le_bytes();
    const END: [u8; 4] = 0x0605_4b50_u32.to_le_bytes();
    // not a tolerance: the size of the ZIP end record without its comment.
    const END_RECORD: usize = 22;
    let not_a_package = || WorkshopPackageError::NotAPackage;
    let bytes = |at: usize, len: usize| {
        at.checked_add(len)
            .and_then(|end| zip.get(at..end))
            .ok_or_else(not_a_package)
    };
    let u16_at =
        |at| bytes(at, 2).map(|field| usize::from(u16::from_le_bytes([field[0], field[1]])));
    let u32_at =
        |at| bytes(at, 4).map(|field| u32::from_le_bytes([field[0], field[1], field[2], field[3]]));
    let usize_at = |at| {
        usize::try_from(u32_at(at)?)
            .map_err(|_out_of_range: std::num::TryFromIntError| WorkshopPackageError::NotAPackage)
    };
    let last = zip
        .len()
        .checked_sub(END_RECORD)
        .ok_or_else(not_a_package)?;
    let end = (last.saturating_sub(usize::from(u16::MAX))..=last)
        .rev()
        .find(|&at| zip[at..].starts_with(&END))
        .ok_or_else(not_a_package)?;
    let count = u16_at(end + 10)?;
    if u16_at(end + 4)? != 0 || u16_at(end + 6)? != 0 || u16_at(end + 8)? != count {
        return Err(not_a_package());
    }
    let central_start = usize_at(end + 16)?;
    if central_start.checked_add(usize_at(end + 12)?) != Some(end)
        || end + END_RECORD + u16_at(end + 20)? != zip.len()
    {
        return Err(WorkshopPackageError::HiddenData);
    }
    let mut at = central_start;
    let mut entries = BTreeMap::new();
    let mut folded_names = BTreeSet::new();
    let mut spans = Vec::with_capacity(count);
    for _ in 0..count {
        if bytes(at, 4)? != CENTRAL {
            return Err(not_a_package());
        }
        let flags = u16_at(at + 8)?;
        let encrypted = flags & 1 != 0;
        let stored = u16_at(at + 10)? == 0;
        let crc = u32_at(at + 16)?;
        let size = usize_at(at + 24)?;
        if encrypted || !stored || usize_at(at + 20)? != size {
            return Err(not_a_package());
        }
        let name_len = u16_at(at + 28)?;
        let raw_name = bytes(at + 46, name_len)?;
        let name = std::str::from_utf8(raw_name)
            .map_err(WorkshopPackageError::NameNotText)?
            .to_owned();
        let local = usize_at(at + 42)?;
        at += 46 + name_len + u16_at(at + 30)? + u16_at(at + 32)?;
        if bytes(local, 4)? != LOCAL {
            return Err(not_a_package());
        }
        let same_header = u16_at(local + 6)? == flags
            && u16_at(local + 8)? == 0
            && u32_at(local + 14)? == crc
            && usize_at(local + 18)? == size
            && usize_at(local + 22)? == size
            && u16_at(local + 26)? == name_len
            && bytes(local + 30, name_len)? == raw_name;
        if !same_header {
            return Err(WorkshopPackageError::LocalHeaderDiffers(name));
        }
        let start = local + 30 + name_len + u16_at(local + 28)?;
        let data = bytes(start, size)?;
        if crate::three_mf_export::crc32(data) != crc {
            return Err(not_a_package());
        }
        if !is_inside(&name) {
            return Err(WorkshopPackageError::BadName(name));
        }
        if !name.is_ascii() && flags & usize::from(crate::three_mf_export::UTF8_NAME) == 0 {
            return Err(WorkshopPackageError::NameNotMarkedUtf8(name));
        }
        if !folded_names.insert(folded(&name)) {
            return Err(WorkshopPackageError::DuplicateName(name));
        }
        spans.push((local, start + size));
        entries.insert(name, data.to_vec());
    }
    if at != end {
        return Err(WorkshopPackageError::HiddenData);
    }
    // The entries follow one another from the start of the archive to the
    // central directory, with no gap a hidden entry could sit in.
    spans.sort_unstable();
    let mut covered = 0;
    for (start, entry_end) in spans {
        if start != covered {
            return Err(WorkshopPackageError::HiddenData);
        }
        covered = entry_end;
    }
    if covered != central_start {
        return Err(WorkshopPackageError::HiddenData);
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "workshop_package_tests.rs"]
mod tests;
