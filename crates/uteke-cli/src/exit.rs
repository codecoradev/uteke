//! The one mapping from an error class to the CLI's wire shape (#1752): the
//! exit status and the line printed on stderr.
//!
//! The CLI has no documented exit codes beyond 0 (success), 1 (failure) and
//! 130 (interrupted), and scripts rely on that, so every class exits 1 and
//! prints `Error: <message>`. Distinct codes per class would be a new public
//! contract and are left out on purpose; this function is where they would go.

use uteke_core::{Error, ErrorClass};

/// The exit status of a failed command.
pub(crate) const EXIT_FAILURE: i32 = 1;

/// What the CLI does about a failed command.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CliFailure {
    pub exit_code: i32,
    pub message: String,
}

/// The exit status and stderr line for a core error, by its class.
pub(crate) fn cli_failure(e: &Error) -> CliFailure {
    let exit_code = match e.class() {
        ErrorClass::BadRequest
        | ErrorClass::NotFound
        | ErrorClass::SecretDetected
        | ErrorClass::RejectedValue
        | ErrorClass::InvalidRoomId
        | ErrorClass::Busy
        | ErrorClass::Embedding
        | ErrorClass::Internal => EXIT_FAILURE,
    };
    CliFailure {
        exit_code,
        message: format!("Error: {e}"),
    }
}

/// A failure that is already a message (a command's `Err(String)`): exit 1.
pub(crate) fn message_failure(message: &str) -> CliFailure {
    CliFailure {
        exit_code: EXIT_FAILURE,
        message: format!("Error: {message}"),
    }
}

/// Print the failure on stderr and exit.
pub(crate) fn exit_with(failure: CliFailure) -> ! {
    eprintln!("{}", failure.message);
    std::process::exit(failure.exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every error variant exits 1 with `Error: <display>`; a class added to
    /// `ErrorClass` fails to compile in `cli_failure` until it is placed.
    #[test]
    fn every_variant_exits_1_with_its_message() {
        let all = Error::samples();
        assert!(all.len() >= 11);
        for e in all {
            let want = format!("Error: {e}");
            assert_eq!(
                cli_failure(&e),
                CliFailure {
                    exit_code: 1,
                    message: want
                },
                "{:?}",
                e.class()
            );
        }
        assert_eq!(message_failure("x").message, "Error: x");
    }
}
