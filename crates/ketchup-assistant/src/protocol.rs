//! The numbers and names the Assistant sidecar protocol shares with
//! `sdk/python/ketchup_assistant_protocol.py`. Each one is defined here once, under the
//! name Python uses too; the Python file carries a block generated from this table, and
//! `python_protocol_block_matches_rust` fails whenever the two differ.

/// A value Python can spell as a literal in the generated block.
pub trait PythonLiteral {
    fn python_literal(&self) -> String;
}

impl PythonLiteral for u16 {
    fn python_literal(&self) -> String {
        self.to_string()
    }
}

impl PythonLiteral for usize {
    fn python_literal(&self) -> String {
        self.to_string()
    }
}

impl PythonLiteral for &str {
    fn python_literal(&self) -> String {
        serde_json::to_string(self).expect("a string always serializes")
    }
}

macro_rules! protocol_constants {
    ($($(#[$meta:meta])* $name:ident: $ty:ty = $value:expr;)*) => {
        $($(#[$meta])* pub const $name: $ty = $value;)*

        /// Every shared constant as `(name, Python literal)`, in definition order.
        pub fn python_constants() -> Vec<(&'static str, String)> {
            vec![$((stringify!($name), $name.python_literal()),)*]
        }
    };
}

protocol_constants! {
    PROTOCOL_VERSION: u16 = 3;
    /// Longest name (part, operation, parameter path, load case), in UTF-8 bytes.
    MAX_NAME_BYTES: usize = ketchup_tolerance::limits::NAME_BYTES;
    /// Longest free text (classification category, occurrence name in context), in UTF-8 bytes.
    MAX_TEXT_BYTES: usize = ketchup_tolerance::limits::TEXT_BYTES;
    /// Longest provider model id, in bytes (ASCII only).
    MAX_MODEL_BYTES: usize = 128;
    /// Longest newline-terminated JSON line either side writes, in bytes.
    MAX_LINE_BYTES: usize = 256 * 1024;
    /// Longest chat message, and longest serialized document context, in characters.
    MAX_MESSAGE_CHARS: usize = 32 * 1024;
    PROJECT_MEMORY_SCHEMA: &str = "ketchup.project-memory.v1";
    /// Most project-memory entries retrieved into one request.
    MAX_PROJECT_MEMORY_ENTRIES: usize = 4;
    /// Most project-memory entries the app keeps.
    MAX_PROJECT_MEMORY_STORED_ENTRIES: usize = 128;
    /// Longest user or assistant text of one project-memory entry, in bytes.
    MAX_PROJECT_MEMORY_TEXT_BYTES: usize = 1024;
    /// Largest serialized project-memory retrieval in one request, in bytes.
    MAX_PROJECT_MEMORY_CONTEXT_BYTES: usize = 8 * 1024;
    MAX_INSPECT_OCCURRENCES: usize = 32;
    MAX_MEASURE_OCCURRENCES: usize = 8;
    MAX_ARRAY_OCCURRENCES: usize = 8;
    MAX_CAD_EDIT_OPERATIONS: usize = 64;
    MAX_CAD_SELECTOR_TARGETS: usize = 100;
    MAX_CAD_GENERATED_OCCURRENCES: usize = 512;
    MAX_VALIDATORS: usize = 32;
    MAX_VALIDATOR_ISSUES: usize = 32;
    MAX_VALIDATOR_ASSUMPTIONS: usize = 8;
    /// Largest serialized result of one local read-only tool call, in bytes.
    MAX_INSPECT_RESULT_BYTES: usize = 64 * 1024;
    /// Most sequential local tool calls the model may make for one request.
    MAX_INSPECT_ROUNDS: usize = 2;
    /// Context key of the host-built catalog the local read-only tools answer from;
    /// the sidecar strips it before the provider sees the context.
    LOCAL_INSPECTION_CATALOG: &str = "_local_inspection_catalog";
}

pub const PYTHON_BLOCK_BEGIN: &str = "# BEGIN generated from crates/ketchup-assistant/src/protocol.rs; do not edit.\n# Regenerate: set KETCHUP_UPDATE_GOLDEN=1 and run cargo test -p ketchup-assistant --lib protocol\n";
pub const PYTHON_BLOCK_END: &str = "# END generated\n";

/// The Python block, markers included.
pub fn python_constants_block() -> String {
    let mut block = String::from(PYTHON_BLOCK_BEGIN);
    for (name, literal) in python_constants() {
        block.push_str(&format!("{name} = {literal}\n"));
    }
    block.push_str(PYTHON_BLOCK_END);
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn python_protocol_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../sdk/python/ketchup_assistant_protocol.py")
    }

    #[test]
    fn python_protocol_block_matches_rust() {
        let path = python_protocol_path();
        let raw = std::fs::read_to_string(&path).expect("read the Python protocol");
        // Git checks the file out with the platform's line endings; compare in LF.
        let crlf = raw.contains("\r\n");
        let source = raw.replace("\r\n", "\n");
        let start = source
            .find(PYTHON_BLOCK_BEGIN)
            .expect("the Python protocol carries the generated block");
        let end = source[start..]
            .find(PYTHON_BLOCK_END)
            .map(|offset| start + offset + PYTHON_BLOCK_END.len())
            .expect("the generated block ends");
        let expected = python_constants_block();
        if source[start..end] == expected {
            return;
        }
        if ketchup_test_env::update_golden() {
            let mut updated = format!("{}{}{}", &source[..start], expected, &source[end..]);
            if crlf {
                updated = updated.replace('\n', "\r\n");
            }
            std::fs::write(&path, updated).expect("write the Python protocol");
            return;
        }
        panic!(
            "{} differs from crates/ketchup-assistant/src/protocol.rs; regenerate it with \
             KETCHUP_UPDATE_GOLDEN=1 cargo test -p ketchup-assistant --lib protocol\n\
             expected:\n{expected}",
            path.display()
        );
    }

    #[test]
    fn python_literals_are_python_syntax() {
        assert_eq!(PROTOCOL_VERSION.python_literal(), "3");
        assert_eq!(MAX_LINE_BYTES.python_literal(), "262144");
        assert_eq!(
            LOCAL_INSPECTION_CATALOG.python_literal(),
            "\"_local_inspection_catalog\""
        );
    }
}
