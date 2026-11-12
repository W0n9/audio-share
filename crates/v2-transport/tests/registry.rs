//! The §6 error registry has exactly one source of truth: `codes::ErrorCode`.
//! `schema::Status::registry` must project its `(retryable, requires_repairing)`
//! columns unchanged, so a new code is added in one place only.

use v2_transport::codes::ErrorCode;
use v2_transport::schema::Status;

#[test]
fn status_registry_projects_the_error_code_table() {
    for entry in ErrorCode::ALL {
        let status = Status::registry(entry.code);
        assert_eq!(
            (status.retryable, status.requires_repairing),
            (entry.retryable, entry.requires_repairing),
            "code {} ({}) disagrees with the single registry table",
            entry.code,
            entry.name()
        );
    }
}

#[test]
fn unknown_codes_have_no_registry_defaults() {
    let status = Status::registry(999);
    assert!(!status.retryable);
    assert!(!status.requires_repairing);
}
