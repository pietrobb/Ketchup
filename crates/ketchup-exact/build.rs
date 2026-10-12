use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// The functions third_party/crmath/ketchup_crmath.c replaces.
const CRMATH_FUNCTIONS: [&str; 26] = [
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "asinh", "acosh",
    "atanh", "exp", "exp2", "exp10", "log", "log2", "log10", "log1p", "pow", "cbrt", "hypot",
    "acosf", "powf", "tanf",
];

fn copy_runtime_dlls(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("failed to create OCCT runtime destination");
    for entry in fs::read_dir(source).expect("failed to read frozen OCCT runtime directory") {
        let entry = entry.expect("failed to inspect frozen OCCT runtime entry");
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        {
            let destination_path = destination.join(entry.file_name());
            if let (Ok(source_bytes), Ok(destination_bytes)) =
                (fs::read(&path), fs::read(&destination_path))
                && source_bytes == destination_bytes
            {
                continue;
            }
            fs::copy(&path, destination_path).expect("failed to stage frozen OCCT runtime DLL");
        }
    }
}

fn main() {
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let repository_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("exact crate must remain below crates/");
    let occt_root = env::var_os("KETCHUP_OCCT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root.join("third_party/occt-install-r0-v1"));
    let backend_fingerprint = env::var("KETCHUP_OCCT_BUILD_FINGERPRINT")
        .unwrap_or_else(|_| "occt-8.0.1:b8f597c677811d1f9f4d8a97f5ae2825c0353a42:r0-v1".to_owned());
    // Windows installs use OCCT's "Windows" layout; other platforms use the "Unix" layout.
    let windows = env::var("CARGO_CFG_WINDOWS").is_ok();
    let (include_dir, library_dir, runtime_dir) = if windows {
        (
            occt_root.join("inc"),
            occt_root.join("win64/vc14/lib"),
            occt_root.join("win64/vc14/bin"),
        )
    } else {
        let library_dir = occt_root.join("lib");
        (
            occt_root.join("include/opencascade"),
            library_dir.clone(),
            library_dir,
        )
    };

    println!("cargo:rustc-env=KETCHUP_OCCT_BUILD_FINGERPRINT={backend_fingerprint}");
    println!("cargo:rerun-if-env-changed=KETCHUP_OCCT_ROOT");
    println!("cargo:rerun-if-env-changed=KETCHUP_OCCT_BUILD_FINGERPRINT");

    for required in [&include_dir, &library_dir, &runtime_dir] {
        assert!(
            required.is_dir(),
            "missing frozen OCCT path: {}",
            required.display()
        );
    }

    let mut native_units: Vec<PathBuf> = fs::read_dir(manifest_dir.join("src"))
        .expect("failed to list native sources")
        .map(|entry| entry.expect("failed to inspect native source").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "cc"))
        .collect();
    native_units.sort();
    let mut native = cxx_build::bridge("src/lib.rs");
    native
        .files(&native_units)
        .include("include")
        .include(&include_dir)
        .std("c++17")
        .flag_if_supported("/EHsc")
        // No fused multiply-add: results must match MSVC bit for bit (see
        // scripts/macos/build-occt.sh).
        .flag_if_supported("-ffp-contract=off");
    // Math calls go to the correctly rounded crmath below; the compiler must
    // not fold or fuse them with its own host libm.
    for function in CRMATH_FUNCTIONS {
        native.flag_if_supported(format!("-fno-builtin-{function}"));
    }
    native.compile("ketchup_exact_native");

    println!("cargo:rustc-link-search=native={}", library_dir.display());
    // Correctly rounded libm (third_party/crmath), installed with OCCT by the
    // build-occt script. Linked whole into every binary that uses this crate:
    // a definition inside the binary wins over the system libm, so Rust's
    // f64::sin etc. on the exact path give the same bits on macOS and Windows,
    // like OCCT itself.
    let crmath = library_dir.join(if windows {
        "ketchup_crmath.lib"
    } else {
        "libketchup_crmath.a"
    });
    assert!(
        crmath.is_file(),
        "missing {}: rebuild OCCT with scripts/*/build-occt",
        crmath.display()
    );
    println!("cargo:rerun-if-changed={}", crmath.display());
    println!("cargo:rustc-link-lib=static:+whole-archive,-bundle=ketchup_crmath");
    for library in [
        "TKernel",
        "TKMath",
        "TKG2d",
        "TKG3d",
        "TKGeomBase",
        "TKBRep",
        "TKGeomAlgo",
        "TKTopAlgo",
        "TKMesh",
        "TKPrim",
        "TKBool",
        "TKBO",
        "TKFillet",
        "TKOffset",
        "TKLCAF",
        "TKCAF",
        "TKXCAF",
        "TKXSBase",
        "TKDESTEP",
        "TKDEIGES",
    ] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("missing OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("unexpected Cargo OUT_DIR layout");
    if windows {
        copy_runtime_dlls(&runtime_dir, profile_dir);
        copy_runtime_dlls(&runtime_dir, &profile_dir.join("deps"));
    } else {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", runtime_dir.display());
    }

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=include/ketchup_exact.hxx");
}
