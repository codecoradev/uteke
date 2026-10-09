//! `docker-entrypoint.sh` embedding-backend handling (#1398).
//!
//! The script is run for real with a fake `curl` (records that a download was
//! attempted) and a fake `uteke-serve` (proves the script reached `exec`).

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "uteke_entrypoint_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::create_dir_all(root.join("data")).unwrap();
        // Fake curl: record the attempt and fail like an offline host.
        write_exec(
            &root.join("bin/curl"),
            &format!(
                "#!/bin/sh\necho \"$@\" >> {}/curl.log\nexit 22\n",
                root.display()
            ),
        );
        // Fake server: proves the script reached `exec uteke-serve`.
        write_exec(
            &root.join("bin/uteke-serve"),
            &format!(
                "#!/bin/sh\necho \"started $@\" > {}/served.log\n",
                root.display()
            ),
        );
        Self { root }
    }

    fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    fn run(&self, env: &[(&str, &str)]) -> Output {
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docker-entrypoint.sh");
        let mut cmd = Command::new("sh");
        cmd.arg(script)
            .arg("--port")
            .arg("1")
            .env_clear()
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.root.join("bin").display()),
            )
            .env("UTEKE_HOME", self.data());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().expect("spawn entrypoint")
    }

    fn downloaded(&self) -> bool {
        self.root.join("curl.log").exists()
    }

    fn served(&self) -> bool {
        self.root.join("served.log").exists()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::set_permissions(self.data(), fs::Permissions::from_mode(0o755));
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_exec(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn external_backend_from_env_skips_the_download_and_starts_the_server() {
    for backend in ["openai", "ollama"] {
        let sb = Sandbox::new();
        let out = sb.run(&[("UTEKE_EMBEDDING_BACKEND", backend)]);
        assert!(out.status.success(), "{}", text(&out));
        assert!(!sb.downloaded(), "{backend}: must not download");
        assert!(sb.served(), "{backend}: must reach exec");
        assert!(text(&out).contains("skipping"), "{}", text(&out));
        assert!(!sb.data().join("models").exists(), "no model dir created");
    }
}

#[test]
fn external_backend_from_uteke_toml_skips_the_download() {
    let sb = Sandbox::new();
    fs::write(
        sb.data().join("uteke.toml"),
        "[server]\nbackend = \"onnx\"\n\n[embedding]\n# comment\nbackend = \"openai\"  # external\nmodel = \"m\"\n",
    )
    .unwrap();
    let out = sb.run(&[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(!sb.downloaded());
    assert!(sb.served());
}

#[test]
fn env_overrides_the_toml_backend() {
    let sb = Sandbox::new();
    fs::write(
        sb.data().join("uteke.toml"),
        "[embedding]\nbackend = \"openai\"\n",
    )
    .unwrap();
    // env says onnx, the model is missing and curl is offline: it must try to
    // download (and fail), proving the env value won over the file.
    let out = sb.run(&[("UTEKE_EMBEDDING_BACKEND", "onnx")]);
    assert!(sb.downloaded(), "{}", text(&out));
    assert!(!out.status.success());
}

#[test]
fn only_the_embedding_section_decides() {
    let sb = Sandbox::new();
    // `backend` under another section must not be mistaken for the embedder.
    fs::write(
        sb.data().join("uteke.toml"),
        "[vector]\nbackend = \"vecq\"\n",
    )
    .unwrap();
    let out = sb.run(&[]);
    assert!(sb.downloaded(), "onnx default expected: {}", text(&out));
}

#[test]
fn onnx_with_the_model_present_does_not_download() {
    let sb = Sandbox::new();
    let onnx = sb.data().join("models/embeddinggemma-q4/onnx");
    fs::create_dir_all(&onnx).unwrap();
    fs::write(onnx.join("model_q4.onnx"), b"x").unwrap();
    let out = sb.run(&[]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(!sb.downloaded());
    assert!(sb.served());
}

#[test]
fn unwritable_data_dir_fails_with_a_clear_message_instead_of_looping_silently() {
    let sb = Sandbox::new();
    fs::set_permissions(sb.data(), fs::Permissions::from_mode(0o555)).unwrap();
    // root ignores directory permissions; nothing to assert there.
    let writable_anyway = fs::create_dir(sb.data().join("probe")).is_ok();
    if writable_anyway {
        return;
    }
    let out = sb.run(&[]);
    assert!(!out.status.success());
    assert!(!sb.served());
    let msg = text(&out);
    assert!(msg.contains("not writable"), "{msg}");
    assert!(msg.contains("UTEKE_EMBEDDING_BACKEND"), "{msg}");
}

#[test]
fn unwritable_data_dir_is_fine_for_an_external_backend() {
    let sb = Sandbox::new();
    fs::set_permissions(sb.data(), fs::Permissions::from_mode(0o555)).unwrap();
    let out = sb.run(&[("UTEKE_EMBEDDING_BACKEND", "openai")]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(sb.served());
}
