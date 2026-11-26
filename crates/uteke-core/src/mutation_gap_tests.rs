//! Tests that pin behaviour the pre-release mutation gate showed to be unchecked
//! (v0.22.0): each fails when the code it names is mutated.

use crate::test_support::open_keyword;

/// `set_author_if_valid` stamps a valid author and leaves an invalid one off,
/// without failing the write that already succeeded.
#[test]
fn set_author_if_valid_records_only_a_valid_author() {
    let u = open_keyword();
    let id = u.remember("who wrote this", &[], None, None).unwrap();
    assert!(u.set_author_if_valid(&id, "alice").unwrap());
    assert_eq!(
        u.get_by_id(&id).unwrap().unwrap().author.as_deref(),
        Some("alice")
    );

    assert!(!u.set_author_if_valid(&id, "").unwrap());
    assert!(!u.set_author_if_valid(&id, &"x".repeat(129)).unwrap());
    assert_eq!(
        u.get_by_id(&id).unwrap().unwrap().author.as_deref(),
        Some("alice"),
        "an invalid author must not overwrite the recorded one"
    );
}

/// A requested contradiction check runs when there is an embedding, and only
/// when it was requested.
#[test]
fn the_contradiction_check_runs_when_requested_and_only_then() {
    let u = open_keyword();
    let first = u
        .remember("alpha beta gamma delta epsilon", &[], None, None)
        .unwrap();

    // Requested, similar text (cosine 0.8, below the duplicate gate): the old
    // memory is found and named.
    let (_, found) = u
        .remember_with_contradiction(
            "alpha beta gamma delta zeta",
            &[],
            None,
            None,
            None,
            true,
            0.65,
        )
        .unwrap();
    assert!(found.contradicted, "{found:?}");
    assert_eq!(found.deprecated_id.as_deref(), Some(first.as_str()));

    // Not requested: nothing is checked, nothing is reported.
    let second = open_keyword();
    second
        .remember("alpha beta gamma delta epsilon", &[], None, None)
        .unwrap();
    let (_, none) = second
        .remember_with_contradiction(
            "alpha beta gamma delta zeta",
            &[],
            None,
            None,
            None,
            false,
            0.65,
        )
        .unwrap();
    assert!(
        !none.contradicted && none.deprecated_id.is_none(),
        "{none:?}"
    );
}

/// `StoreSettings::load` reads the global `uteke.toml` of `UTEKE_HOME`.
#[test]
#[serial_test::serial]
#[allow(unsafe_code)] // env mutation, serialized with the other env tests
fn store_settings_load_reads_the_global_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("uteke.toml"),
        "[embedding]\nbackend = \"ollama\"\n[vector]\nbackend = \"vecq\"\n",
    )
    .unwrap();
    let previous = std::env::var("UTEKE_HOME").ok();
    unsafe { std::env::set_var("UTEKE_HOME", dir.path()) };
    unsafe { std::env::remove_var("UTEKE_EMBEDDING_BACKEND") };
    let s = crate::embedding_config::StoreSettings::load();
    match previous {
        Some(v) => unsafe { std::env::set_var("UTEKE_HOME", v) },
        None => unsafe { std::env::remove_var("UTEKE_HOME") },
    }
    assert_eq!(s.embedding.backend, "ollama");
    assert_eq!(s.vector.backend, "vecq");
}

/// Puts the named environment variables back to what they were, also when an
/// assertion fails, so one test cannot leak a value into the others of this
/// binary. Create it before the first `set` and keep it alive to the end.
struct EnvRestore(Vec<(&'static str, Option<String>)>);

impl EnvRestore {
    fn new(names: &[&'static str]) -> Self {
        Self(names.iter().map(|n| (*n, std::env::var(n).ok())).collect())
    }

    #[allow(unsafe_code)] // serialized with the other env tests
    fn set(&self, name: &str, value: &str) {
        unsafe { std::env::set_var(name, value) };
    }

    #[allow(unsafe_code)] // serialized with the other env tests
    fn unset(&self, name: &str) {
        unsafe { std::env::remove_var(name) };
    }
}

impl Drop for EnvRestore {
    #[allow(unsafe_code)] // serialized with the other env tests
    fn drop(&mut self) {
        for (name, previous) in &self.0 {
            match previous {
                Some(v) => unsafe { std::env::set_var(name, v) },
                None => unsafe { std::env::remove_var(name) },
            }
        }
    }
}

/// `privacy::load_config` reads `[privacy]` from the global `uteke.toml` of
/// `UTEKE_HOME`, and `UTEKE_PRIVACY_MODE` wins over the file. A load that
/// returned the default would give `redact` and no allowlist.
#[test]
#[serial_test::serial]
fn privacy_load_config_reads_the_global_file_then_the_environment() {
    use crate::privacy::{PrivacyMode, load_config};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("uteke.toml"),
        "[privacy]\nmode = \"strict\"\nallowlist = [\"^KEEP-[0-9]+$\"]\n",
    )
    .unwrap();
    let env = EnvRestore::new(&["UTEKE_HOME", "UTEKE_PRIVACY_MODE"]);
    env.set("UTEKE_HOME", dir.path().to_str().unwrap());
    env.unset("UTEKE_PRIVACY_MODE");

    let from_file = load_config();
    assert_eq!(from_file.mode, PrivacyMode::Strict);
    assert_eq!(from_file.allowlist.len(), 1);
    assert!(from_file.allowlist[0].is_match("KEEP-42"));
    assert!(!from_file.allowlist[0].is_match("DROP-42"));

    env.set("UTEKE_PRIVACY_MODE", "off");
    let from_env = load_config();
    assert_eq!(from_env.mode, PrivacyMode::Off, "the environment wins");
    assert_eq!(from_env.allowlist.len(), 1, "the file's allowlist stays");
}

/// `EngineSettings::load` reads the global `uteke.toml` of `UTEKE_HOME` and
/// then applies the process environment. A load that returned the default
/// would give a recall floor of 0.0 and no fallback model.
#[test]
#[serial_test::serial]
fn engine_settings_load_reads_the_global_file_then_the_environment() {
    use crate::EngineSettings;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("uteke.toml"),
        "[recall]\nmin_score = 0.2\n[embed_fallback]\napi_key = \"file-key\"\n\
         base_url = \"https://file.test\"\nmodel = \"file-model\"\n",
    )
    .unwrap();
    let env = EnvRestore::new(&[
        "UTEKE_HOME",
        "UTEKE_RECALL_MIN_SCORE",
        "UTEKE_EMBED_FALLBACK_MODEL",
    ]);
    env.set("UTEKE_HOME", dir.path().to_str().unwrap());
    env.unset("UTEKE_RECALL_MIN_SCORE");
    env.unset("UTEKE_EMBED_FALLBACK_MODEL");

    let from_file = EngineSettings::load();
    assert_eq!(from_file.recall.min_score, 0.2);
    assert_eq!(from_file.embed_fallback.model, "file-model");
    assert_eq!(from_file.embed_fallback.base_url, "https://file.test");

    env.set("UTEKE_RECALL_MIN_SCORE", "0.6");
    env.set("UTEKE_EMBED_FALLBACK_MODEL", "env-model");
    let from_env = EngineSettings::load();
    assert_eq!(from_env.recall.min_score, 0.6, "the environment wins");
    assert_eq!(from_env.embed_fallback.model, "env-model");
    assert_eq!(
        from_env.embed_fallback.base_url, "https://file.test",
        "a key the environment does not set keeps the file value"
    );
}

/// `EngineSettings::with_env_overrides` applies the real process environment
/// on top of the value it is called on and keeps everything else: a body that
/// returned the default would drop the file value of `[recall] min_score`.
#[test]
#[serial_test::serial]
fn engine_settings_with_env_overrides_keeps_the_rest_and_applies_the_environment() {
    use crate::EngineSettings;
    let env = EnvRestore::new(&["UTEKE_RECALL_MIN_SCORE", "UTEKE_EMBED_FALLBACK_MODEL"]);
    env.unset("UTEKE_RECALL_MIN_SCORE");
    env.set("UTEKE_EMBED_FALLBACK_MODEL", "env-model");

    let mut file = EngineSettings::default();
    file.recall.min_score = 0.3;
    let got = file.with_env_overrides();
    assert_eq!(got.recall.min_score, 0.3, "unset variables keep the value");
    assert_eq!(got.embed_fallback.model, "env-model");
}

/// `Uteke::list_pinned` (#1804) hands back the pinned memories of the store it
/// wraps: the pinned ones only, filtered by namespace, within the limit. The
/// store method has its own tests; this one fails when the wrapper answers
/// without asking the store.
#[test]
fn uteke_list_pinned_returns_the_pinned_memories_of_the_store() {
    let u = open_keyword();
    let a = u
        .remember("pinned in alpha", &[], None, Some("alpha"))
        .unwrap();
    let b = u
        .remember("pinned in beta", &[], None, Some("beta"))
        .unwrap();
    let plain = u
        .remember("never pinned", &[], None, Some("alpha"))
        .unwrap();
    assert!(u.pin(&a).unwrap());
    assert!(u.pin(&b).unwrap());

    let mut all: Vec<String> = u
        .list_pinned(None, 10)
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    all.sort();
    let mut expected = vec![a.clone(), b.clone()];
    expected.sort();
    assert_eq!(all, expected, "every pinned memory, and no plain one");
    assert!(!all.contains(&plain));

    let in_alpha = u.list_pinned(Some("alpha"), 10).unwrap();
    assert_eq!(in_alpha.len(), 1);
    assert_eq!(in_alpha[0].id, a);
    assert!(in_alpha[0].pinned);

    assert_eq!(
        u.list_pinned(None, 1).unwrap().len(),
        1,
        "the limit applies"
    );
}
