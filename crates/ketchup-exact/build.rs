use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn copy_runtime_dlls(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("failed to create OCCT runtime destination");
    for entry in fs::read_dir(source).expect("failed to read frozen OCCT runtime directory") {
        let entry = entry.expect("failed to inspect frozen OCCT runtime entry");
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
        {
            fs::copy(&path, destination.join(entry.file_name()))
                .expect("failed to stage frozen OCCT runtime DLL");
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
    let include_dir = occt_root.join("inc");
    let library_dir = occt_root.join("win64/vc14/lib");
    let runtime_dir = occt_root.join("win64/vc14/bin");

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

    cxx_build::bridge("src/lib.rs")
        .file("src/native.cc")
        .include("include")
        .include(&include_dir)
        .std("c++17")
        .flag_if_supported("/EHsc")
        .compile("ketchup_exact_native");

    println!("cargo:rustc-link-search=native={}", library_dir.display());
    for library in [
        "TKernel",
        "TKMath",
        "TKG2d",
        "TKG3d",
        "TKGeomBase",
        "TKBRep",
        "TKGeomAlgo",
        "TKTopAlgo",
        "TKPrim",
        "TKBool",
        "TKBO",
        "TKXSBase",
        "TKDESTEP",
    ] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }

    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("missing OUT_DIR"));
    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("unexpected Cargo OUT_DIR layout");
    copy_runtime_dlls(&runtime_dir, profile_dir);
    copy_runtime_dlls(&runtime_dir, &profile_dir.join("deps"));

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/native.cc");
    println!("cargo:rerun-if-changed=include/ketchup_exact.hxx");
}
