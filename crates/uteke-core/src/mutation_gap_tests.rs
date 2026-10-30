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
