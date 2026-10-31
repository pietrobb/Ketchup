//! The Android side of `ketchup-viewer --benchmark` (V9): a development
//! install pushes `benchmark.ketchup-view` next to `model.ketchup-view`; the
//! next start measures it on the window's GPU, writes `benchmark.txt` beside
//! it with the peak memory added, and removes the request so later starts are
//! normal. scripts/windows/benchmark-android-viewer.ps1 drives it.

use std::path::{Path, PathBuf};

/// The package a development install asked to be measured, if any.
pub fn request(external: &Path) -> Option<PathBuf> {
    Some(external.join("benchmark.ketchup-view")).filter(|path| path.is_file())
}

/// Where the report of `request` is written.
pub fn report_path(request: &Path) -> PathBuf {
    request.with_file_name("benchmark.txt")
}

/// Peak resident memory in bytes from `/proc/self/status` (`VmHWM`).
pub fn peak_memory_bytes(status: &str) -> Option<u64> {
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))?;
    let kib = line.trim().strip_suffix("kB")?.trim().parse::<u64>().ok()?;
    kib.checked_mul(1024)
}

/// The Viewer report with the peak memory line the Windows script adds.
pub fn report_with_peak(report: &str, peak: Option<u64>) -> String {
    let peak = peak.map_or_else(
        || "peak memory: unknown".to_owned(),
        |bytes| format!("peak memory: {} MB", bytes / (1024 * 1024)),
    );
    format!("{report}{peak}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_pushed_benchmark_package_is_a_request() {
        let dir = std::env::temp_dir().join(format!("ketchup-benchmark-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(request(&dir), None);
        std::fs::write(dir.join("model.ketchup-view"), b"x").unwrap();
        assert_eq!(request(&dir), None, "the shown model is not measured");
        let pushed = dir.join("benchmark.ketchup-view");
        std::fs::write(&pushed, b"x").unwrap();
        assert_eq!(request(&dir), Some(pushed.clone()));
        assert_eq!(report_path(&pushed), dir.join("benchmark.txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn peak_memory_is_read_from_the_high_water_mark() {
        let status = "Name:\tsk.ketchup.viewer\nVmPeak:\t 9999999 kB\nVmHWM:\t  204800 kB\nVmRSS:\t  102400 kB\n";
        assert_eq!(peak_memory_bytes(status), Some(204_800 * 1024));
        assert_eq!(peak_memory_bytes("VmRSS:\t 1 kB\n"), None);
        assert_eq!(peak_memory_bytes("VmHWM:\t lots\n"), None);
    }

    #[test]
    fn the_report_ends_with_the_peak_memory_like_on_windows() {
        let report = report_with_peak("open: 2.0 ms\n", Some(200 * 1024 * 1024));
        assert_eq!(report, "open: 2.0 ms\npeak memory: 200 MB\n");
        assert!(report_with_peak("", None).contains("unknown"));
    }
}
