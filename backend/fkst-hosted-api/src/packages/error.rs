//! Package domain errors and MongoDB duplicate-key detection.

use mongodb::error::{ErrorKind, WriteFailure};

/// Errors surfaced by the packages domain.
///
/// Conceptual HTTP mapping (the `IntoResponse` rendering lives in the API
/// layer): `Validation` -> 400, `Duplicate` -> 409, `Db` -> 500.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    /// The input failed `NewPackage::validate`. The message is the stable,
    /// user-facing validation reason.
    #[error("invalid package: {0}")]
    Validation(String),
    /// A package with this name already exists (Mongo `_id` collision,
    /// duplicate-key code 11000). Carries the offending package name.
    #[error("package already exists: {0}")]
    Duplicate(String),
    /// Any other MongoDB failure. The `Display` is deliberately the static
    /// `"database error"` — the driver text may carry host/connection
    /// details and must never reach a client. The underlying error stays
    /// reachable via `source()` for ERROR-level logging only.
    #[error("database error")]
    Db(#[from] mongodb::error::Error),
}

/// True when `code` is MongoDB's duplicate-key error code (E11000).
fn code_is_dup(code: i32) -> bool {
    code == 11000
}

/// True when the driver error is a duplicate-key (`_id` collision) failure.
///
/// `insert_one` normally surfaces the single-write path
/// (`ErrorKind::Write(WriteFailure::WriteError)` with code 11000); the
/// `BulkWrite` and `Command` arms are defensive because driver versions
/// differ in how they report the failure.
#[allow(dead_code)] // consumed by PackageRepository::create (next commit)
pub(crate) fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    match &*err.kind {
        ErrorKind::Write(WriteFailure::WriteError(write_error)) => code_is_dup(write_error.code),
        ErrorKind::BulkWrite(bulk) => bulk
            .write_errors
            .values()
            .any(|write_error| code_is_dup(write_error.code)),
        ErrorKind::Command(command_error) => code_is_dup(command_error.code),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use bson::doc;
    use mongodb::error::{BulkWriteError, CommandError, WriteError};

    use super::*;

    /// Build a driver `WriteError`. The struct is `#[non_exhaustive]`, so
    /// tests construct it through its public `Deserialize` impl instead of a
    /// struct literal.
    fn write_error(code: i32) -> WriteError {
        bson::from_document(doc! { "code": code, "errmsg": "boom" }).expect("WriteError shape")
    }

    fn write_failure_error(code: i32) -> mongodb::error::Error {
        mongodb::error::Error::from(ErrorKind::Write(WriteFailure::WriteError(write_error(
            code,
        ))))
    }

    #[test]
    fn code_is_dup_only_for_11000() {
        assert!(code_is_dup(11000));
        assert!(!code_is_dup(0));
        assert!(!code_is_dup(121));
        assert!(!code_is_dup(11001));
    }

    #[test]
    fn is_duplicate_key_detects_write_error_11000() {
        assert!(is_duplicate_key(&write_failure_error(11000)));
        assert!(!is_duplicate_key(&write_failure_error(121)));
    }

    #[test]
    fn is_duplicate_key_detects_command_error_11000() {
        let command: CommandError = bson::from_document(
            doc! { "code": 11000, "codeName": "DuplicateKey", "errmsg": "dup" },
        )
        .expect("CommandError shape");
        let err = mongodb::error::Error::from(ErrorKind::Command(command));
        assert!(is_duplicate_key(&err));
    }

    #[test]
    fn is_duplicate_key_detects_bulk_write_error_11000() {
        let mut bulk = BulkWriteError::default();
        bulk.write_errors.insert(0, write_error(11000));
        let err = mongodb::error::Error::from(ErrorKind::BulkWrite(bulk));
        assert!(is_duplicate_key(&err));
    }

    #[test]
    fn is_duplicate_key_is_false_for_non_write_kinds() {
        let io = std::io::Error::other("connection refused");
        assert!(!is_duplicate_key(&mongodb::error::Error::from(io)));
    }

    #[test]
    fn db_display_is_static_and_never_leaks_driver_text() {
        let io = std::io::Error::other("dial mongodb://user:secret@db:27017 refused");
        let err = PackageError::Db(mongodb::error::Error::from(io));
        assert_eq!(err.to_string(), "database error");
        // The source remains reachable for server-side logging only.
        let source = std::error::Error::source(&err).expect("source preserved");
        assert!(source.to_string().contains("secret"));
    }

    #[test]
    fn validation_and_duplicate_display_texts_are_stable() {
        assert_eq!(
            PackageError::Validation("bad".to_string()).to_string(),
            "invalid package: bad"
        );
        assert_eq!(
            PackageError::Duplicate("demo".to_string()).to_string(),
            "package already exists: demo"
        );
    }
}
