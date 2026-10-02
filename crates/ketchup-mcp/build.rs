//! Embeds every example program (`examples/programs/*.star`) for the program docs.
use std::{env, fs, io, path::PathBuf};

fn main() -> io::Result<()> {
    let examples = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("../../examples/programs")
        .canonicalize()?;
    println!("cargo:rerun-if-changed={}", examples.display());
    let mut files: Vec<PathBuf> = fs::read_dir(&examples)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<_>>()?;
    files.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "star")
    });
    files.sort();
    let mut code = String::from("const EXAMPLES: &[(&str, &str)] = &[\n");
    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        code.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    code.push_str("];\n");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap_or_default());
    fs::write(out.join("examples.rs"), code)
}
