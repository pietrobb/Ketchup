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
    // Written by hand with a decimal comma, it fell back to 35 mm (review
    // 2026-11-07 r26 P3).
    if let Some(drill) = last(LARGEST_DRILL_MM)
        .and_then(|text| text.trim().replace(',', ".").parse::<f64>().ok())
        .filter(|drill| (0.0..=80.0).contains(drill))
    {
        settings.largest_drill_mm = drill;
    }
    settings
}

/// Each key of `settings` and its value as kept.
fn keyed(settings: &BtlxSettings) -> [(&'static str, String); 3] {
    let profile = match settings.profile_strategy {
        BtlxProfileStrategy::PortableFreeContour => PORTABLE,
        BtlxProfileStrategy::EdgeSawCutsThenMillContour => SAW_THEN_MILL,
    };
    [
        (PROFILE, profile.to_owned()),
        (
            INTERMEDIATE_SAW_CUTS,
            settings.intermediate_saw_cuts.to_string(),
        ),
        (LARGEST_DRILL_MM, settings.largest_drill_mm.to_string()),
    ]
}

/// Keep `settings` in `file`, all at one turn of its lock.
#[cfg(test)]
fn save(file: &SettingsFile, settings: &BtlxSettings) -> std::io::Result<()> {
    file.replace_each(&keyed(settings))
}

impl KetchupApp {
    /// Keep the BTLx options for the next start on a worker, as the lock of
    /// the settings may be waited for (review 2026-11-07 r26 P3: three times
    /// on the window). Changed again while a worker keeps them, they are kept
    /// once it is done, so an older value is not kept last.
    pub(super) fn remember_btlx_settings(&mut self) {
        let saving = &mut self.window.btlx_saving;
        if saving.worker.is_some() {
            saving.again = true;
            return;
        }
        let Some(file) = self.window.btlx_settings.clone() else {
            return;
        };
        let keyed = keyed(&self.window.btlx);
        let context = self.file.activity.context.clone();
        saving.worker = Some(std::thread::spawn(move || {
            let saved = file.replace_each(&keyed);
            if let Some(context) = context {
                context.request_repaint();
            }
            saved
        }));
    }

    /// Once the worker keeping the BTLx options is done, tells the digest
    /// when they could not be kept, and keeps them again if they changed.
    pub(super) fn poll_btlx_saving(&mut self) {
        let saving = &mut self.window.btlx_saving;
        if saving
            .worker
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished)
            && let Some(worker) = saving.worker.take()
        {
            self.btlx_kept(worker);
        }
    }

    /// Joins `worker`, which kept the BTLx options, telling the digest when it
    /// could not, and keeps them again if they changed meanwhile.
    fn btlx_kept(&mut self, worker: std::thread::JoinHandle<std::io::Result<()>>) {
        let saved = worker
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        let again = std::mem::take(&mut self.window.btlx_saving.again);
        if let Err(error) = saved {
            self.digest = self.catalog.format(
                "digest-btlx-options-not-remembered",
                &BTreeMap::from([("error", error.to_string())]),
            );
        }
        if again {
            self.remember_btlx_settings();
        }
    }

    /// Waits for the worker keeping the BTLx options, and for the one keeping
    /// them again after it.
    #[cfg(test)]
    pub(crate) fn btlx_saved(&mut self) {
        while let Some(worker) = self.window.btlx_saving.worker.take() {
            self.btlx_kept(worker);
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

    /// Review 2026-11-07 r26 P3: each option waited for the lock of the
    /// settings on its own, and a drill written with a decimal comma was
    /// back at 35 mm.
    #[test]
    fn the_options_are_kept_at_one_turn_and_a_decimal_comma_is_read() {
        let folder = tempfile::tempdir().unwrap();
        let file = SettingsFile::at(folder.path().join(FILE_NAME));
        let turn = || std::fs::read_to_string(folder.path().join("btlx.txt.turn")).unwrap();
        save(&file, &BtlxSettings::default()).unwrap();
        let before: u64 = turn().parse().unwrap();
        save(&file, &BtlxSettings::default()).unwrap();
        assert_eq!(turn().parse::<u64>().unwrap(), before + 1);

        std::fs::write(folder.path().join(FILE_NAME), "largest-drill-mm=20,5\n").unwrap();
        assert_eq!(read(&file).largest_drill_mm, 20.5);
    }

    /// Review 2026-11-07 r26 P3: the options were kept on the window, which
    /// waited for the lock. A change while a worker keeps them is kept after
    /// it, and one not kept is told.
    #[test]
    fn the_options_are_kept_on_a_worker_and_again_when_changed_meanwhile() {
        let folder = tempfile::tempdir().unwrap();
        let file = SettingsFile::at(folder.path().join(FILE_NAME));
        let mut app = KetchupApp::new();
        app.window.btlx_settings = Some(file.clone());
        let (release, held) = std::sync::mpsc::channel::<()>();
        app.window.btlx_saving.worker = Some(std::thread::spawn(move || {
            held.recv().ok();
            Ok(())
        }));
        app.window.btlx.largest_drill_mm = 20.5;
        app.remember_btlx_settings();
        assert!(app.window.btlx_saving.again);
        assert!(file.values(LARGEST_DRILL_MM).is_empty());
        release.send(()).unwrap();
        app.btlx_saved();
        assert_eq!(read(&file).largest_drill_mm, 20.5);

        app.window.btlx_saving.worker = Some(std::thread::spawn(|| {
            Err(std::io::Error::other("the disk is full"))
        }));
        app.btlx_saved();
        assert!(app.digest.contains("the disk is full"), "{}", app.digest);
    }
}
