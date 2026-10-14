import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "check_gpu_test_instances", ROOT / "scripts" / "check_gpu_test_instances.py"
)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


def write(root, relative, text):
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_the_viewer_tests_that_corrupted_the_heap_fail(tmp_path):
    # Before #103: two tests each made an instance (one through egui_kittest).
    write(tmp_path, "crates/ketchup-viewer/src/gpu_tests.rs",
          "fn t() { let i = wgpu::Instance::new(&d); }\n")
    write(tmp_path, "crates/ketchup-viewer/src/app/tests.rs",
          "fn t() { let s = egui_kittest::wgpu::create_render_state(setup()); }\n")
    assert checker.current_counts(tmp_path) == {
        "crates/ketchup-viewer/src/app/tests.rs": 1,
        "crates/ketchup-viewer/src/gpu_tests.rs": 1,
    }
    assert checker.check(tmp_path) == 1


def test_the_shared_instance_and_production_code_pass(tmp_path):
    write(tmp_path, "crates/ketchup-test-gpu/src/lib.rs",
          "pub fn instance() { block_on(default_wgpu_setup().new_instance()) }\n")
    write(tmp_path, "crates/ketchup-viewer/src/gpu_tests.rs",
          "fn t() { let (device, queue) = ketchup_test_gpu::gpu_device(); }\n")
    write(tmp_path, "crates/ketchup-viewer/src/desktop.rs",
          "fn run() { let i = wgpu::Instance::new(&d); }\n")
    assert checker.check(tmp_path) == 0


def test_a_rendered_harness_needs_an_explicit_renderer(tmp_path):
    write(tmp_path, "crates/ketchup-app/tests/image.rs",
          "fn t() {\n"
          "    let mut h = Harness::builder()\n        .with_size(s)\n"
          "        .build_state(|c, a| a.ui(c), app);\n"
          "    h.render().unwrap();\n"
          "    let mut quiet = Harness::builder().build_state(|c, a| a.ui(c), app);\n"
          "    quiet.step();\n"
          "    let doc = store.snapshot();\n"
          "}\n")
    assert checker.current_counts(tmp_path) == {"crates/ketchup-app/tests/image.rs": 1}
    write(tmp_path, "crates/ketchup-app/tests/image.rs",
          "fn t() {\n"
          "    let mut h = Harness::builder()\n        .renderer(ketchup_test_gpu::renderer())\n"
          "        .build_state(|c, a| a.ui(c), app);\n"
          "    h.render().unwrap();\n"
          "}\n")
    assert checker.check(tmp_path) == 0


def test_the_repository_creates_no_instances_in_tests():
    assert checker.check(ROOT) == 0
