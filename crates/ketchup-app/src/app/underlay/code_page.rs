//! The code page of a drawing read as an underlay, when the one its header names is
//! doubtful (review 2026-11-09 UL-303): „Dĺžka steny, Hĺbka základu“ in Windows-1250 under
//! a header naming ANSI_1252 stays in 1252, „Håbka“ being a Swedish word. Before it is
//! added, the window shows how its texts read and lets the user keep that or read the
//! drawing again in another code page, which the underlay keeps for writing it back.

use super::ReadUnderlay;
use crate::*;
use ketchup_model::import::DxfCodePage;

/// How many of the drawing's texts beyond ASCII the window shows.
const SHOWN: usize = 6;

/// A drawing read whose code page is doubtful, waiting for the user.
pub(crate) struct CodePageChoice {
    path: PathBuf,
    unit: Option<ImportLengthUnit>,
    read: Box<ReadUnderlay>,
    /// The code page its header names, doubted.
    doubted: String,
    /// The code page it was read in.
    read_in: String,
    /// The code page to read it again in.
    chosen: DxfCodePage,
    /// Its layer names and texts beyond ASCII as read, the first [`SHOWN`].
    samples: Vec<String>,
}

impl CodePageChoice {
    pub(super) fn new(
        path: PathBuf,
        unit: Option<ImportLengthUnit>,
        read: ReadUnderlay,
        doubted: String,
    ) -> Self {
        let read_in = read
            .read
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code() == "dxf.text-code-page")
            .and_then(|diagnostic| diagnostic.subject())
            .unwrap_or("UTF-8")
            .to_owned();
        // The page read in is offered last: another is what the user may want.
        let chosen = DxfCodePage::ALL
            .into_iter()
            .find(|page| page.name() != read_in && page.name() != doubted)
            .unwrap_or(DxfCodePage::ALL[0]);
        let mut samples = Vec::new();
        let texts = read
            .lines
            .layers()
            .iter()
            .flat_map(|layer| layer.texts().iter().map(|text| text.text.as_str()));
        for text in read.read.layers.iter().map(String::as_str).chain(texts) {
            if samples.len() == SHOWN {
                break;
            }
            if !text.is_ascii() && !samples.iter().any(|sample| sample == text) {
                samples.push(text.to_owned());
            }
        }
        Self {
            path,
            unit,
            read: Box::new(read),
            doubted,
            read_in,
            chosen,
            samples,
        }
    }
}

/// The code page `read`'s header names where it is doubtful.
pub(super) fn doubted(read: &ReadUnderlay) -> Option<String> {
    read.read
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == "dxf.text-code-page-doubtful")
        .and_then(|diagnostic| diagnostic.subject())
        .map(str::to_owned)
}

impl KetchupApp {
    /// Shows how a drawing whose code page is doubtful reads, and keeps it so or reads it
    /// again in the code page the user chooses.
    pub(crate) fn show_underlay_code_page_window(&mut self, context: &egui::Context) {
        let Some(mut choice) = self.file.activity.underlay.code_page_choice.take() else {
            return;
        };
        let (mut again, mut keep, mut cancel) = (false, false, false);
        egui::Window::new(self.catalog.text("dialog-underlay-code-page-title"))
            .id(egui::Id::new("underlay-code-page"))
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-underlay-code-page-text",
                    &BTreeMap::from([
                        ("path", choice.path.display().to_string()),
                        ("named", choice.doubted.clone()),
                        ("read", choice.read_in.clone()),
                    ]),
                ));
                for sample in &choice.samples {
                    ui.monospace(sample);
                }
                egui::ComboBox::from_label(self.catalog.text("dialog-underlay-code-page-choose"))
                    .selected_text(choice.chosen.name())
                    .show_ui(ui, |ui| {
                        for page in DxfCodePage::ALL {
                            ui.selectable_value(&mut choice.chosen, page, page.name());
                        }
                    });
                ui.horizontal(|ui| {
                    again = ui
                        .button(self.catalog.text("dialog-underlay-code-page-read-again"))
                        .clicked();
                    keep = ui
                        .button(self.catalog.text("dialog-underlay-code-page-keep"))
                        .clicked();
                    cancel = ui.button(self.catalog.text("action-cancel")).clicked();
                });
            });
        if again {
            self.start_underlay_read(choice.path, choice.unit, Some(choice.chosen));
        } else if keep {
            if let Err(error) = self.add_underlay(&choice.path, *choice.read) {
                self.report_import_failure(&choice.path, &error);
            }
        } else if cancel {
            self.digest = self.catalog.text("digest-cancelled");
        } else {
            self.file.activity.underlay.code_page_choice = Some(choice);
        }
    }
}
