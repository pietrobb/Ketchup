//! The "send to workshop" package (review 2026-11-08 §5.2): the shop files of a
//! model in one stored ZIP with a report of what could not be made and a
//! SHA-256 list, and the acceptance check the workshop runs on what it got.

use crate::fabrication::{
    BtlxExportOptions, GeneralFabricationError, GeneralFabricationProjection, ShopFormat, ShopLoss,
    WoodwopMprOptions,
};
use ketchup_model::document::Snapshot;
use ketchup_model::exact_product::ExactProductError;
use ketchup_model::graph::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};
use std::str::Utf8Error;
use std::sync::Arc;

// not a tolerance: the length of a SHA-256 written in hex.
const SHA256_HEX_LEN: usize = 64;

/// The SHA-256 list, in the format `sha256sum -c` reads.
pub const WORKSHOP_SUMS: &str = "SHA256SUMS";
/// What the package could not hold, and why.
pub const WORKSHOP_REPORT: &str = "not_made.txt";
/// What the files of the package are written without, or a machine may not
/// make, one per line: `output,part,element,reason` (review 2026-11-10
/// MF-400, MF-401).
pub const WORKSHOP_LOSSES: &str = "losses.csv";

/// A shop output the package does not hold, and why.
#[derive(Clone, Debug)]
pub struct NotMade {
    pub output: String,
    pub reason: Arc<dyn std::error::Error + Send + Sync>,
}

/// An element of a part a file of the package is written without, or that
/// a machine may not make from it: a thread, a hole of a part the shop does
/// not make, a round hole milled rather than drilled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lost {
    pub output: String,
    pub part: String,
    pub element: String,
    pub reason: String,
}

/// The files to send, by their path in the package, the outputs that could
/// not be made and what the made ones lose.
#[derive(Clone, Debug, Default)]
pub struct WorkshopPackage {
    pub files: Vec<(String, Vec<u8>)>,
    pub not_made: Vec<NotMade>,
    pub losses: Vec<Lost>,
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
    /// No output could be made, each `<output>: <reason>` (review
    /// 2026-11-10 MF-406).
    NothingMade(Vec<String>),
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
}

impl std::fmt::Display for WorkshopPackageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("there is no shop file to send"),
            Self::NothingMade(reasons) => write!(
                formatter,
                "no shop file could be made: {}",
                reasons.join("; ")
            ),
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
        }
    }
}

impl std::error::Error for WorkshopPackageError {}

impl WorkshopPackage {
    /// The package as a stored ZIP: the files, [`WORKSHOP_REPORT`],
    /// [`WORKSHOP_LOSSES`] and, last, [`WORKSHOP_SUMS`] with the hash of each.
    ///
    /// # Errors
    /// [`WorkshopPackageError::NothingMade`] with every reason when no output
    /// could be made, and [`WorkshopPackageError::Empty`] when there was none.
    pub fn zip(&self) -> Result<Vec<u8>, WorkshopPackageError> {
        if self.files.is_empty() {
            return Err(if self.not_made.is_empty() {
                WorkshopPackageError::Empty
            } else {
                WorkshopPackageError::NothingMade(
                    self.not_made
                        .iter()
                        .map(|item| format!("{}: {}", item.output, item.reason))
                        .collect(),
                )
            });
        }
        let mut names = BTreeSet::new();
        for (name, _) in &self.files {
            if !is_inside(name)
                || [WORKSHOP_SUMS, WORKSHOP_REPORT, WORKSHOP_LOSSES].contains(&&**name)
            {
                return Err(WorkshopPackageError::BadName(name.clone()));
            }
            if !names.insert(name.as_str()) {
                return Err(WorkshopPackageError::DuplicateName(name.clone()));
            }
        }
        let report = report_text(&self.not_made);
        let losses = losses_csv(&self.losses);
        let mut entries = self
            .files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect::<Vec<_>>();
        entries.push((WORKSHOP_REPORT, report.as_bytes()));
        entries.push((WORKSHOP_LOSSES, losses.as_bytes()));
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
    /// part drawings, BTLx with its support report and the woodWOP program
    /// and label of each piece. An output that cannot be made is named with
    /// its reason instead, and does not stop the others; what a made one
    /// loses is listed in [`WorkshopPackage::losses`].
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
        // BTLx with the support report its own export writes beside it, and
        // its losses as rows of the package's list.
        let btlx = self
            .btlx_2_3_1_export_with_options(snapshot, options.btlx)
            .and_then(|btlx| {
                let support = self.btlx_support_report(snapshot, options.btlx)?;
                Ok((btlx, support, btlx_losses(self, snapshot, options.btlx)?))
            });
        let mut losses = match btlx {
            Ok((btlx, support, losses)) => {
                add(
                    "BTLx",
                    Ok(vec![
                        ("btlx/model.btlx".to_owned(), btlx),
                        (
                            "btlx/model.btlx.support.txt".to_owned(),
                            support.into_bytes(),
                        ),
                    ]),
                );
                losses
            }
            Err(error) => {
                add("BTLx", Err(error));
                Vec::new()
            }
        };
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
                let woodwop = ShopFormat::Woodwop {
                    vertical_pocket_tool: options.woodwop.vertical_pocket_tool_number.is_some(),
                };
                losses.extend(
                    self.unmade_pin_holes(snapshot, woodwop)
                        .into_iter()
                        .map(|loss| shop_lost("woodWOP", loss)),
                );
            }
            Err(error) => add("woodWOP", Err(error)),
        }
        package.losses = losses;
        package
    }
}

fn shop_lost(output: &str, loss: ShopLoss) -> Lost {
    Lost {
        output: output.to_owned(),
        part: loss.part,
        element: loss.element.to_string(),
        reason: loss.reason.to_owned(),
    }
}

/// What BTLx written with `options` loses: the elements its gate passes at
/// a loss, and each round hole it mills rather than drills (review
/// 2026-11-10 MF-401).
fn btlx_losses(
    projection: &GeneralFabricationProjection,
    snapshot: &Snapshot,
    options: BtlxExportOptions,
) -> Result<Vec<Lost>, GeneralFabricationError> {
    let mut losses = projection
        .admit(snapshot, ShopFormat::Btlx)?
        .into_iter()
        .map(|loss| shop_lost("BTLx", loss))
        .collect::<Vec<_>>();
    let largest = options.largest_drill_mm;
    let reason = largest.map_or_else(
        || {
            "the largest drill of the machine is not set, so a round opening is milled along \
             its outline"
                .to_owned()
        },
        |largest| format!("it is larger than the largest drill, Ø{largest}, so it is milled"),
    );
    losses.extend(
        projection
            .btlx_milled_round_holes(snapshot, largest)
            .into_iter()
            .map(|(part, diameter_mm)| Lost {
                output: "BTLx".to_owned(),
                part,
                element: format!("a round hole Ø{diameter_mm}"),
                reason: reason.clone(),
            }),
    );
    Ok(losses)
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

/// The losses as CSV with a header, every field quoted.
fn losses_csv(losses: &[Lost]) -> String {
    let field = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
    std::iter::once("output,part,element,reason\n".to_owned())
        .chain(losses.iter().map(|lost| {
            [&lost.output, &lost.part, &lost.element, &lost.reason]
                .map(|text| field(text))
                .join(",")
                + "\n"
        }))
        .collect()
}

/// A plain relative path that stays inside the folder the package is
/// unpacked into, on every system.
fn is_inside(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', ':'])
        && !name.chars().any(char::is_control)
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
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
/// points and checked against its CRC.
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
    let mut at = usize_at(end + 16)?;
    let mut entries = BTreeMap::new();
    for _ in 0..count {
        if bytes(at, 4)? != CENTRAL {
            return Err(not_a_package());
        }
        let encrypted = u16_at(at + 8)? & 1 != 0;
        let stored = u16_at(at + 10)? == 0;
        let crc = u32_at(at + 16)?;
        let size = usize_at(at + 24)?;
        if encrypted || !stored || usize_at(at + 20)? != size {
            return Err(not_a_package());
        }
        let name_len = u16_at(at + 28)?;
        let name = std::str::from_utf8(bytes(at + 46, name_len)?)
            .map_err(WorkshopPackageError::NameNotText)?
            .to_owned();
        let local = usize_at(at + 42)?;
        at += 46 + name_len + u16_at(at + 30)? + u16_at(at + 32)?;
        if bytes(local, 4)? != LOCAL {
            return Err(not_a_package());
        }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let data = bytes(start, size)?;
        if crate::three_mf_export::crc32(data) != crc {
            return Err(not_a_package());
        }
        if !is_inside(&name) {
            return Err(WorkshopPackageError::BadName(name));
        }
        if entries.contains_key(&name) {
            return Err(WorkshopPackageError::DuplicateName(name));
        }
        entries.insert(name, data.to_vec());
    }
    Ok(entries)
}

#[cfg(test)]
#[path = "workshop_package_tests.rs"]
mod tests;
