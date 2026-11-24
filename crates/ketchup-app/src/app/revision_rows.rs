//! The revisions of the drawing set in the drawings window (review 2026-11-08
//! §5.3): index, date and what changed, oldest first, which every sheet lists
//! beside its title block.

use eframe::egui;
use ketchup_interaction::LocaleCatalog;
use ketchup_manufacturing::revision_table::SheetRevision;
use ketchup_manufacturing::title_block::SheetSettings;

/// Shows the rows of `settings.revisions` with a remove button each and an
/// add button.
pub(crate) fn show_revisions(
    ui: &mut egui::Ui,
    catalog: &LocaleCatalog,
    settings: &mut SheetSettings,
) {
    ui.vertical(|ui| {
        if settings.revisions.is_empty() {
            ui.weak(catalog.text("drawings-revisions-none"));
        }
        let mut removed = None;
        for (index, revision) in settings.revisions.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut revision.index)
                        .hint_text(catalog.text("drawings-sheet-revision-index"))
                        .desired_width(40.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut revision.date)
                        .hint_text(catalog.text("drawings-sheet-revision-date"))
                        .desired_width(80.0),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut revision.change)
                        .hint_text(catalog.text("drawings-sheet-revision-change"))
                        .desired_width(240.0),
                );
                if ui
                    .button(catalog.text("drawings-revision-remove"))
                    .clicked()
                {
                    removed = Some(index);
                }
            });
        }
        if let Some(index) = removed {
            settings.revisions.remove(index);
        }
        if ui.button(catalog.text("drawings-revision-add")).clicked() {
            let index = settings
                .revisions
                .last()
                .map_or_else(|| "A".to_owned(), |last| next_index(&last.index));
            settings.revisions.push(SheetRevision {
                index,
                ..SheetRevision::default()
            });
        }
    });
}

/// The index after `index`: the next letter after a single letter, the next
/// number of as many digits after a number; empty after anything else.
fn next_index(index: &str) -> String {
    let index = index.trim();
    let mut characters = index.chars();
    if let (Some(letter), None) = (characters.next(), characters.next())
        && letter.is_ascii_alphabetic()
        && !letter.eq_ignore_ascii_case(&'z')
        && let Some(next) = char::from_u32(u32::from(letter) + 1)
    {
        return next.to_string();
    }
    if !index.is_empty()
        && index.chars().all(|digit| digit.is_ascii_digit())
        && let Some(next) = index.parse::<u64>().ok().and_then(|n| n.checked_add(1))
    {
        return format!("{next:0width$}", width = index.len());
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_revision_is_added_with_the_index_after_the_last() {
        assert_eq!(next_index("A"), "B");
        assert_eq!(next_index("c"), "d");
        assert_eq!(next_index("01"), "02");
        assert_eq!(next_index("9"), "10");
        assert_eq!(next_index("Z"), "");
        assert_eq!(next_index("R1"), "");
        assert_eq!(next_index(""), "");
    }
}
