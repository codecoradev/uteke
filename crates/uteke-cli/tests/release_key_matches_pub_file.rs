//! The key embedded in `uteke upgrade` must equal `scripts/uteke-release.pub`.
//!
//! `uteke-cli` is a binary crate, so the constant is not importable: this test
//! checks that the key line of the pub file appears in `RELEASE_PUBLIC_KEYS`.
//! The script lives outside the crate directory, so `include_str!` would break
//! `cargo package`/`cargo publish`; it is read at runtime instead and the test
//! is skipped (with a message) when the file is absent.

use std::path::Path;

#[test]
fn embedded_release_key_matches_scripts_pub_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let pub_path = root.join("../../scripts/uteke-release.pub");
    let Ok(text) = std::fs::read_to_string(&pub_path) else {
        eprintln!("skipped: {} not found", pub_path.display());
        return;
    };
    let key_line = text.lines().nth(1).expect("pub file has a key line").trim();
    assert!(!key_line.is_empty());

    let src = std::fs::read_to_string(root.join("src/commands/upgrade.rs")).unwrap();
    let decl = src
        .split("const RELEASE_PUBLIC_KEYS")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .expect("RELEASE_PUBLIC_KEYS declaration");
    assert!(
        decl.contains(&format!("\"{key_line}\"")),
        "embedded key does not match scripts/uteke-release.pub"
    );
}
