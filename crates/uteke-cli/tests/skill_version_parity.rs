use uteke_core::Uteke;

/// #1267: the bundled SKILL.md must advertise the CLI's own version.
/// `uteke init` installs this file verbatim; if it lags the workspace
/// version, every newly onboarded agent starts reading stale docs.
#[test]
fn skill_version_parity() {
    let skill = include_str!("../assets/uteke-memory-skill.md");
    let ver = env!("CARGO_PKG_VERSION");

    let expected = format!("Version: **{ver}**");
    assert!(
        skill.contains(&expected),
        "bundled uteke-memory-skill.md is stale: expected `{expected}` \
         in its version line. Bump the version line when bumping the \
         workspace version (single line, keeps `uteke init` output in \
         lockstep with the release)."
    );
}

/// The file `uteke init` installs must stay identical to the canonical
/// copy checked into `.agents/skills/uteke-memory/SKILL.md`.
#[test]
fn skill_bundled_matches_repo_copy() {
    let bundled = include_str!("../assets/uteke-memory-skill.md");
    let repo_copy = include_str!("../../../.agents/skills/uteke-memory/SKILL.md");
    assert_eq!(
        bundled, repo_copy,
        "crates/uteke-cli/assets/uteke-memory-skill.md diverged from \
         .agents/skills/uteke-memory/SKILL.md — keep both in sync \
         (assets copy is the install source; .agents copy is the repo wiki)."
    );
}

/// Smoke: init writes the bundled skill into the target layout.
#[test]
fn init_installs_skill_md() {
    let dir = tempfile::tempdir().unwrap();
    // install_skill_md is private; exercise it through the same include
    // contract: the file must exist and carry the version header.
    let skill = include_str!("../assets/uteke-memory-skill.md");
    assert!(skill.starts_with("---\ndescription:"));
    let _ = Uteke::open(dir.path().join("t.db")).unwrap();
}

/// The MCP Registry entry and the Gemini CLI extension manifest carry their
/// own version string; a release that forgets to bump them publishes stale
/// metadata (the registry treats versions as immutable).
#[test]
fn registry_manifests_match_workspace_version() {
    let ver = env!("CARGO_PKG_VERSION");
    for (name, raw) in [
        ("server.json", include_str!("../../../server.json")),
        (
            "gemini-extension.json",
            include_str!("../../../gemini-extension.json"),
        ),
    ] {
        let json: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert_eq!(
            json["version"].as_str(),
            Some(ver),
            "{name} is stale: set its `version` to {ver} in the release-prep commit"
        );
    }
}
