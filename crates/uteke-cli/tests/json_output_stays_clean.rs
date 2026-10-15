//! `--json` output must be parseable JSON on stdout, whatever the logs say.
//!
//! The CLI console log layer used to write to stdout, so any WARN printed before
//! the command's own output and broke `json.loads(stdout)` (#1461). Diagnostics
//! belong on stderr.

use std::process::Command;

/// Run `uteke --store <tmp> --json remember ...` with an ONNX library path that
/// does not exist: embedding fails, which logs WARN/ERROR lines, but the
/// memory is still stored and the command prints its JSON result.
fn remember_with_failing_embedder() -> (String, String) {
    let tmp = std::env::temp_dir().join(format!("uteke_json_clean_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let home = tmp.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // Server routing off, so the command runs against the local store.
    std::fs::write(home.join("uteke.toml"), "[server]\nenabled = false\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_uteke"))
        .args(["--store"])
        .arg(tmp.join("store"))
        .args(["--json", "remember", "json output check"])
        .env("UTEKE_HOME", &home)
        .env("UTEKE_NO_UPDATE_CHECK", "1")
        .env("ORT_LIB_PATH", "/nonexistent/uteke-test/libonnxruntime.so")
        .env_remove("UTEKE_AUTH_TOKEN")
        .output()
        .expect("run uteke");
    let _ = std::fs::remove_dir_all(&tmp);
    assert!(
        out.status.success(),
        "uteke failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn json_stdout_is_clean_when_logs_are_emitted() {
    let (stdout, stderr) = remember_with_failing_embedder();

    let json: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not a single JSON value ({e}): {stdout:?}"));
    assert_eq!(json["embedding_written"], false, "{json}");

    assert!(
        stderr.contains("WARN") || stderr.contains("ERROR"),
        "the embedding failure should be logged on stderr, got: {stderr:?}"
    );
    assert!(
        !stdout.contains("WARN") && !stdout.contains("ERROR"),
        "log lines leaked onto stdout: {stdout:?}"
    );
}
