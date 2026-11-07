//! The BTLx options of the File menu, kept between starts in `btlx.txt` beside
//! the user's other settings (review 2026-11-07 P3: the largest drill went back
//! to 35 mm at each start). A value this window cannot read is left at its
//! default, and a key it does not know kept.

use crate::app_state::BtlxSettings;
use crate::recent_files::user_settings::SettingsFile;
use crate::{BtlxProfileStrategy, KetchupApp};
use std::collections::BTreeMap;

const FILE_NAME: &str = "btlx.txt";
const PROFILE: &str = "profile";
const INTERMEDIATE_SAW_CUTS: &str = "intermediate-saw-cuts";
const LARGEST_DRILL_MM: &str = "largest-drill-mm";
const PORTABLE: &str = "portable-contour";
const SAW_THEN_MILL: &str = "saw-then-mill";

/// The user's file of BTLx options, when the platform names a folder for it.
pub(crate) fn of_user() -> Option<SettingsFile> {
    SettingsFile::of_user_named(FILE_NAME)
}

/// The options kept in `file`, each one not kept or not read at its default.
pub(crate) fn read(file: &SettingsFile) -> BtlxSettings {
    let mut settings = BtlxSettings::default();
    let last = |key| file.values(key).pop();
    match last(PROFILE).as_deref() {
        Some(PORTABLE) => settings.profile_strategy = BtlxProfileStrategy::PortableFreeContour,
        Some(SAW_THEN_MILL) => {
            settings.profile_strategy = BtlxProfileStrategy::EdgeSawCutsThenMillContour;
        }
        _ => {}
    }
    if let Some(cuts) = last(INTERMEDIATE_SAW_CUTS)
        .and_then(|text| text.trim().parse::<u8>().ok())
        .filter(|cuts| *cuts <= 32)
    {
        settings.intermediate_saw_cuts = cuts;
    }
    if let Some(drill) = last(LARGEST_DRILL_MM)
        .and_then(|text| text.trim().parse::<f64>().ok())
        .filter(|drill| (0.0..=80.0).contains(drill))
    {
        settings.largest_drill_mm = drill;
    }
    settings
}

/// Keep `settings` in `file`.
pub(crate) fn save(file: &SettingsFile, settings: &BtlxSettings) -> std::io::Result<()> {
    let profile = match settings.profile_strategy {
        BtlxProfileStrategy::PortableFreeContour => PORTABLE,
        BtlxProfileStrategy::EdgeSawCutsThenMillContour => SAW_THEN_MILL,
    };
    file.replace(PROFILE, &[profile.to_owned()])?;
    file.replace(
        INTERMEDIATE_SAW_CUTS,
        &[settings.intermediate_saw_cuts.to_string()],
    )?;
    file.replace(LARGEST_DRILL_MM, &[settings.largest_drill_mm.to_string()])
}

impl KetchupApp {
    /// Keep the BTLx options for the next start, telling the digest when they
    /// cannot be.
    pub(super) fn remember_btlx_settings(&mut self) {
        let Some(file) = &self.window.btlx_settings else {
            return;
        };
        if let Err(error) = save(file, &self.window.btlx) {
            self.digest = self.catalog.format(
                "digest-btlx-options-not-remembered",
                &BTreeMap::from([("error", error.to_string())]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_btlx_options_are_kept_and_a_value_not_read_is_its_default() {
        let folder = tempfile::tempdir().unwrap();
        let file = SettingsFile::at(folder.path().join(FILE_NAME));
        let defaults = BtlxSettings::default();
        let unchanged = read(&file);
        assert_eq!(unchanged.profile_strategy, defaults.profile_strategy);
        assert_eq!(unchanged.largest_drill_mm, defaults.largest_drill_mm);

        let chosen = BtlxSettings {
            profile_strategy: BtlxProfileStrategy::PortableFreeContour,
            intermediate_saw_cuts: 7,
            largest_drill_mm: 20.0,
        };
        save(&file, &chosen).unwrap();
        let kept = read(&SettingsFile::at(folder.path().join(FILE_NAME)));
        assert_eq!(kept.profile_strategy, chosen.profile_strategy);
        assert_eq!(kept.intermediate_saw_cuts, 7);
        assert_eq!(kept.largest_drill_mm, 20.0);

        std::fs::write(
            folder.path().join(FILE_NAME),
            "profile=lathe\nintermediate-saw-cuts=900\nlargest-drill-mm=NaN\nfuture=1\n",
        )
        .unwrap();
        let unread = read(&file);
        assert_eq!(unread.profile_strategy, defaults.profile_strategy);
        assert_eq!(unread.intermediate_saw_cuts, defaults.intermediate_saw_cuts);
        assert_eq!(unread.largest_drill_mm, defaults.largest_drill_mm);
        save(&file, &chosen).unwrap();
        assert_eq!(file.values("future"), ["1"]);
    }
}
