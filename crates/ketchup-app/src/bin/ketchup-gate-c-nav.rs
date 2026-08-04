#![forbid(unsafe_code)]

use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use ketchup_app::{AdapterRequirement, KetchupApp};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RUNS: usize = 30;
const WARMUP: Duration = Duration::from_secs(10);
const MEASUREMENT: Duration = Duration::from_secs(30);
const OCCURRENCES: usize = 10_000;
const TESSELLATED_TRIANGLES: usize = OCCURRENCES * 2;

fn main() -> eframe::Result {
    if cfg!(debug_assertions) {
        eprintln!("Gate C navigation measurements require a release build");
        std::process::exit(2);
    }
    let arguments = std::env::args().collect::<Vec<_>>();
    assert_eq!(
        arguments.len(),
        6,
        "usage: ketchup-gate-c-nav <profile-id> <series> <r0-lock-sha256> <expected-adapter-name> <output-json>"
    );
    let profile_id = arguments[1].clone();
    assert!(matches!(profile_id.as_str(), "HP-DEV-01" | "HP-IGPU-01"));
    let series = arguments[2]
        .parse::<usize>()
        .expect("series must be a positive integer");
    assert!((1..=3).contains(&series));
    let lock_sha256 = arguments[3].clone();
    assert_eq!(lock_sha256.len(), 64, "R0 lock SHA-256 must be complete");
    let expected_adapter_name = arguments[4].clone();
    assert!(!expected_adapter_name.trim().is_empty());
    let output_path = PathBuf::from(&arguments[5]);
    assert!(!output_path.exists(), "Gate C evidence must be immutable");

    let result = Arc::new(Mutex::new(None));
    let app_result = Arc::clone(&result);
    let selected_adapter = Arc::new(Mutex::new(None));
    let app_selected_adapter = Arc::clone(&selected_adapter);
    let required_device_type = if profile_id == "HP-IGPU-01" {
        eframe::wgpu::DeviceType::IntegratedGpu
    } else {
        eframe::wgpu::DeviceType::DiscreteGpu
    };
    let environment = environment_fingerprint();
    let revision = repository_revision();
    eframe::run_native(
        &KetchupApp::title(),
        KetchupApp::native_options_for_adapter(
            Some(AdapterRequirement {
                name: expected_adapter_name,
                device_type: required_device_type,
            }),
            selected_adapter,
        ),
        Box::new(move |_creation_context| {
            let adapter_info = app_selected_adapter
                .lock()
                .expect("selected-adapter evidence lock must remain available")
                .clone()
                .expect("the adapter selector must record the selected adapter");
            Ok(Box::new(NavigationHarness::new(
                profile_id,
                series,
                lock_sha256,
                output_path,
                environment,
                revision,
                adapter_info,
                app_result,
            )))
        }),
    )?;

    let metrics = result
        .lock()
        .expect("navigation result lock must remain available")
        .clone()
        .expect("navigation harness closed before completing all runs");
    assert!(metrics.frame_p95_ms <= 16.7, "{metrics:?}");
    assert!(metrics.frame_p99_ms <= 33.3, "{metrics:?}");
    assert!(metrics.input_to_preview_p95_ms <= 50.0, "{metrics:?}");
    println!(
        "Gate C navigation series {series} PASS: frame p95 {:.4} ms, frame p99 {:.4} ms, input-to-preview p95 {:.4} ms",
        metrics.frame_p95_ms, metrics.frame_p99_ms, metrics.input_to_preview_p95_ms
    );
    Ok(())
}

struct NavigationHarness {
    profile_id: String,
    series: usize,
    lock_sha256: String,
    output_path: PathBuf,
    environment: String,
    revision: RepositoryRevision,
    adapter_info: eframe::wgpu::AdapterInfo,
    result: Arc<Mutex<Option<Metrics>>>,
    run_index: usize,
    run_started: Instant,
    last_frame_started: Option<Instant>,
    frame_ms: Vec<f64>,
    input_to_preview_ms: Vec<f64>,
    per_run_frame_samples: [usize; RUNS],
    per_run_preview_samples: [usize; RUNS],
}

impl NavigationHarness {
    #[allow(clippy::too_many_arguments)]
    fn new(
        profile_id: String,
        series: usize,
        lock_sha256: String,
        output_path: PathBuf,
        environment: String,
        revision: RepositoryRevision,
        adapter_info: eframe::wgpu::AdapterInfo,
        result: Arc<Mutex<Option<Metrics>>>,
    ) -> Self {
        Self {
            profile_id,
            series,
            lock_sha256,
            output_path,
            environment,
            revision,
            adapter_info,
            result,
            run_index: 0,
            run_started: Instant::now(),
            last_frame_started: None,
            frame_ms: Vec::with_capacity(RUNS * 2_000),
            input_to_preview_ms: Vec::with_capacity(RUNS * 2_000),
            per_run_frame_samples: [0; RUNS],
            per_run_preview_samples: [0; RUNS],
        }
    }

    fn complete_run_or_series(&mut self, context: &egui::Context) {
        self.run_index += 1;
        if self.run_index < RUNS {
            self.run_started = Instant::now();
            self.last_frame_started = None;
            return;
        }

        let metrics = Metrics {
            frame_p95_ms: percentile(&self.frame_ms, 95),
            frame_p99_ms: percentile(&self.frame_ms, 99),
            input_to_preview_p95_ms: percentile(&self.input_to_preview_ms, 95),
        };
        write_metrics(
            &self.output_path,
            &self.profile_id,
            self.series,
            &self.lock_sha256,
            &self.environment,
            &self.revision,
            &self.adapter_info,
            &metrics,
            &self.per_run_frame_samples,
            &self.per_run_preview_samples,
            &self.frame_ms,
            &self.input_to_preview_ms,
        );
        *self
            .result
            .lock()
            .expect("navigation result lock must remain available") = Some(metrics);
        context.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

impl eframe::App for NavigationHarness {
    fn update(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        context.request_repaint();
        let frame_started = Instant::now();
        let elapsed = self.run_started.elapsed();
        if elapsed >= WARMUP + MEASUREMENT {
            self.complete_run_or_series(context);
            return;
        }

        let input_started = Instant::now();
        let seconds = elapsed.as_secs_f32();
        let orbit = seconds * 0.42;
        let pan = Vec2::new((seconds * 0.71).sin() * 12.0, (seconds * 0.53).cos() * 8.0);
        let zoom = 0.92 + (seconds * 0.37).sin() * 0.08;
        let preview_offset = (seconds * 1.7).sin() * 1.5;

        egui::CentralPanel::default().show(context, |ui| {
            let rect = ui.available_rect_before_wrap();
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, Color32::from_rgb(24, 28, 36));
            render_occurrences(&painter, rect, orbit, pan, zoom, preview_offset);
        });

        let previous_frame = self.last_frame_started.replace(frame_started);
        if elapsed >= WARMUP {
            if let Some(previous_frame) = previous_frame {
                self.frame_ms
                    .push(milliseconds(frame_started.duration_since(previous_frame)));
                self.per_run_frame_samples[self.run_index] += 1;
            }
            self.input_to_preview_ms
                .push(milliseconds(input_started.elapsed()));
            self.per_run_preview_samples[self.run_index] += 1;
        }
    }
}

fn render_occurrences(
    painter: &egui::Painter,
    viewport: Rect,
    orbit: f32,
    pan: Vec2,
    zoom: f32,
    preview_offset: f32,
) {
    let center = viewport.center() + pan;
    let spacing = viewport.width().min(viewport.height()) / 118.0 * zoom;
    let (sin, cos) = orbit.sin_cos();
    let size = Vec2::splat((spacing * 0.42).max(1.0));
    for index in 0..OCCURRENCES {
        let grid_x = (index % 100) as f32 - 49.5;
        let grid_y = (index / 100) as f32 - 49.5;
        let x = (grid_x * cos - grid_y * sin) * spacing;
        let y = (grid_x * sin + grid_y * cos) * spacing * 0.62;
        let preview = if index % 97 == 0 { preview_offset } else { 0.0 };
        let occurrence_center = Pos2::new(center.x + x, center.y + y - preview);
        painter.rect_filled(
            Rect::from_center_size(occurrence_center, size),
            0.0,
            Color32::from_rgb(92, 172, 232),
        );
    }
}

fn percentile(samples: &[f64], percentile: usize) -> f64 {
    assert!(!samples.is_empty(), "navigation run produced no samples");
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    let index = (ordered.len() * percentile).div_ceil(100) - 1;
    ordered[index]
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn environment_fingerprint() -> String {
    let script = concat!(
        "$cpu=Get-CimInstance Win32_Processor|Select-Object -First 1 Name,NumberOfCores,NumberOfLogicalProcessors;",
        "$gpu=Get-CimInstance Win32_VideoController|Select-Object Name,DriverVersion,AdapterRAM;",
        "$os=Get-CimInstance Win32_OperatingSystem|Select-Object Caption,Version,BuildNumber,TotalVisibleMemorySize;",
        "[pscustomobject]@{cpu=$cpu;gpu=@($gpu);os=$os}|ConvertTo-Json -Compress -Depth 4"
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .expect("PowerShell must capture the Windows environment");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("environment JSON must be UTF-8")
        .trim()
        .to_owned()
}

#[derive(Clone)]
struct RepositoryRevision {
    git_head: String,
    working_tree_dirty: bool,
}

fn repository_revision() -> RepositoryRevision {
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("Git revision must be available");
    assert!(head.status.success());
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .expect("Git working-tree state must be available");
    assert!(status.status.success());
    RepositoryRevision {
        git_head: String::from_utf8(head.stdout)
            .expect("Git revision must be UTF-8")
            .trim()
            .to_owned(),
        working_tree_dirty: !status.stdout.is_empty(),
    }
}

#[derive(Clone, Debug)]
struct Metrics {
    frame_p95_ms: f64,
    frame_p99_ms: f64,
    input_to_preview_p95_ms: f64,
}

#[allow(clippy::too_many_arguments)]
fn write_metrics(
    output_path: &Path,
    profile_id: &str,
    series: usize,
    lock_sha256: &str,
    environment: &str,
    revision: &RepositoryRevision,
    adapter_info: &eframe::wgpu::AdapterInfo,
    metrics: &Metrics,
    per_run_frame_samples: &[usize; RUNS],
    per_run_preview_samples: &[usize; RUNS],
    frame_ms: &[f64],
    input_to_preview_ms: &[f64],
) {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).expect("Gate C artifact directory must be writable");
    }
    let mut json = String::new();
    writeln!(json, "{{").unwrap();
    writeln!(json, "  \"schema_version\": 1,").unwrap();
    writeln!(json, "  \"query_class\": \"QC-C-NAV-01\",").unwrap();
    writeln!(json, "  \"profile_id\": \"{profile_id}\",").unwrap();
    writeln!(json, "  \"series\": {series},").unwrap();
    writeln!(json, "  \"r0_lock_sha256\": \"{lock_sha256}\",").unwrap();
    writeln!(json, "  \"environment\": {environment},").unwrap();
    write_adapter_info(&mut json, adapter_info);
    writeln!(json, "  \"git_head\": \"{}\",", revision.git_head).unwrap();
    writeln!(
        json,
        "  \"working_tree_dirty\": {},",
        revision.working_tree_dirty
    )
    .unwrap();
    writeln!(json, "  \"runs\": {RUNS},").unwrap();
    writeln!(json, "  \"warmup_seconds_per_run\": {},", WARMUP.as_secs()).unwrap();
    writeln!(
        json,
        "  \"measurement_seconds_per_run\": {},",
        MEASUREMENT.as_secs()
    )
    .unwrap();
    writeln!(json, "  \"occurrences\": {OCCURRENCES},").unwrap();
    writeln!(
        json,
        "  \"visible_tessellated_triangles\": {TESSELLATED_TRIANGLES},"
    )
    .unwrap();
    writeln!(json, "  \"shared_authoritative_geometry\": 1,").unwrap();
    writeln!(json, "  \"frame_p95_ms\": {:.6},", metrics.frame_p95_ms).unwrap();
    writeln!(json, "  \"frame_p99_ms\": {:.6},", metrics.frame_p99_ms).unwrap();
    writeln!(
        json,
        "  \"input_to_preview_p95_ms\": {:.6},",
        metrics.input_to_preview_p95_ms
    )
    .unwrap();
    write_usize_samples(
        &mut json,
        "per_run_frame_sample_counts",
        per_run_frame_samples,
        true,
    );
    write_usize_samples(
        &mut json,
        "per_run_input_to_preview_sample_counts",
        per_run_preview_samples,
        true,
    );
    write_f64_samples(&mut json, "frame_ms", frame_ms, true);
    write_f64_samples(&mut json, "input_to_preview_ms", input_to_preview_ms, false);
    writeln!(json, "}}").unwrap();
    std::fs::write(output_path, json).expect("Gate C raw navigation metrics must be written");
}

fn write_adapter_info(json: &mut String, info: &eframe::wgpu::AdapterInfo) {
    writeln!(json, "  \"selected_adapter\": {{").unwrap();
    write!(json, "    \"name\": ").unwrap();
    write_json_string(json, &info.name);
    writeln!(json, ",").unwrap();
    writeln!(json, "    \"vendor_id\": {},", info.vendor).unwrap();
    writeln!(json, "    \"device_id\": {},", info.device).unwrap();
    writeln!(
        json,
        "    \"device_type\": \"{}\",",
        match info.device_type {
            eframe::wgpu::DeviceType::Other => "other",
            eframe::wgpu::DeviceType::IntegratedGpu => "integrated-gpu",
            eframe::wgpu::DeviceType::DiscreteGpu => "discrete-gpu",
            eframe::wgpu::DeviceType::VirtualGpu => "virtual-gpu",
            eframe::wgpu::DeviceType::Cpu => "cpu",
        }
    )
    .unwrap();
    write!(json, "    \"driver\": ").unwrap();
    write_json_string(json, &info.driver);
    writeln!(json, ",").unwrap();
    write!(json, "    \"driver_info\": ").unwrap();
    write_json_string(json, &info.driver_info);
    writeln!(json, ",").unwrap();
    writeln!(json, "    \"backend\": \"{}\"", info.backend).unwrap();
    writeln!(json, "  }},").unwrap();
}

fn write_json_string(json: &mut String, value: &str) {
    json.push('"');
    for character in value.chars() {
        match character {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            '\u{08}' => json.push_str("\\b"),
            '\u{0c}' => json.push_str("\\f"),
            '\n' => json.push_str("\\n"),
            '\r' => json.push_str("\\r"),
            '\t' => json.push_str("\\t"),
            control if control <= '\u{1f}' => write!(json, "\\u{:04x}", control as u32).unwrap(),
            other => json.push(other),
        }
    }
    json.push('"');
}

fn write_usize_samples(json: &mut String, name: &str, samples: &[usize], comma: bool) {
    write!(json, "  \"{name}\": [").unwrap();
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        write!(json, "{sample}").unwrap();
    }
    writeln!(json, "]{}", if comma { "," } else { "" }).unwrap();
}

fn write_f64_samples(json: &mut String, name: &str, samples: &[f64], comma: bool) {
    write!(json, "  \"{name}\": [").unwrap();
    for (index, sample) in samples.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        write!(json, "{sample:.6}").unwrap();
    }
    writeln!(json, "]{}", if comma { "," } else { "" }).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentile_is_used() {
        let samples = (1..=100).map(|value| value as f64).collect::<Vec<_>>();
        assert_eq!(percentile(&samples, 95), 95.0);
        assert_eq!(percentile(&samples, 99), 99.0);
    }

    #[test]
    fn frozen_scene_stays_below_triangle_cap() {
        assert_eq!(OCCURRENCES, 10_000);
        assert_eq!(TESSELLATED_TRIANGLES, 20_000);
    }

    #[test]
    fn adapter_strings_are_validly_escaped_for_evidence_json() {
        let mut json = String::new();
        write_json_string(&mut json, "GPU \"A\"\nDriver");
        assert_eq!(json, "\"GPU \\\"A\\\"\\nDriver\"");
    }
}
