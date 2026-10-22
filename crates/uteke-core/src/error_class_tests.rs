//! How errors are classified for the transports (#1517).

use crate::test_support::open_keyword;
use crate::{Error, ErrorClass};

#[test]
fn every_variant_has_a_class() {
    let io = Error::Io(std::io::Error::other("x"));
    assert_eq!(io.class(), ErrorClass::Internal);
    assert_eq!(Error::db_msg("x").class(), ErrorClass::Internal);
    assert_eq!(Error::embed_msg("x").class(), ErrorClass::Internal);
    assert_eq!(Error::lock("x").class(), ErrorClass::Internal);
    assert_eq!(Error::generic("x").class(), ErrorClass::Internal);
    assert_eq!(Error::validation("x").class(), ErrorClass::BadRequest);
    assert_eq!(Error::not_found("x").class(), ErrorClass::NotFound);
    let secret = Error::SecretDetected {
        kinds: vec!["jwt".into()],
    };
    assert_eq!(secret.class(), ErrorClass::SecretDetected);
}

#[test]
fn a_not_found_error_shows_its_message_as_it_is() {
    assert_eq!(
        Error::not_found("Room not found: r1").to_string(),
        "Room not found: r1"
    );
    // Validation keeps its prefix: only the new variant is bare.
    assert_eq!(
        Error::validation("bad").to_string(),
        "Validation error: bad"
    );
}

#[test]
fn deleting_a_missing_room_and_moving_a_missing_document_are_not_found() {
    let u = open_keyword();
    let err = u.delete_room("ghost").unwrap_err();
    assert_eq!(err.class(), ErrorClass::NotFound, "{err}");
    assert_eq!(err.to_string(), "Room not found: ghost");

    let err = u.doc_move("no-such-doc", None, None).unwrap_err();
    assert_eq!(err.class(), ErrorClass::NotFound, "{err}");
    assert!(err.to_string().contains("not found"), "{err}");
}

#[test]
fn sites_that_still_answer_400_stay_validation() {
    // The contract with uteke-cloud decides when these become NotFound; until
    // then a missing memory on update and a missing room on rename are 400s.
    let u = open_keyword();
    let err = u
        .update_memory("no-such-memory", Some("x"), None, None, None, None, None)
        .unwrap_err();
    assert_eq!(err.class(), ErrorClass::BadRequest, "{err}");
}
