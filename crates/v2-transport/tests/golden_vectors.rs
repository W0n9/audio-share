//! Golden-vector replay against `fixtures/wire-protocol-v2/` (spec §11.1,
//! ticket 05's replay contract): every control happy/error case (34+3) must
//! round-trip byte-exactly in both directions, with no private encoding.

use v2_transport::vectors::{replay_case, FixtureSet, ReplayReport};

/// The authoritative count from `manifest.json` (`case_counts_by_category`):
/// control-error-registry 26 + control-happy-path 6 + control-negative-space 6
/// + resource-limits 3 = 41 control-plane cases.
fn expected_control_case_count(manifest: &serde_json::Value) -> usize {
    let counts = &manifest["case_counts_by_category"];
    (counts["control-error-registry"].as_u64().unwrap_or(0)
        + counts["control-happy-path"].as_u64().unwrap_or(0)
        + counts["control-negative-space"].as_u64().unwrap_or(0)
        + counts["resource-limits"].as_u64().unwrap_or(0)) as usize
}

#[test]
fn replay_all_control_cases_byte_exactly() {
    let fixtures = FixtureSet::repo_default().expect("fixtures/wire-protocol-v2/ is present");
    let manifest_bytes = std::fs::read(fixtures.root().join("manifest.json")).expect("manifest");
    let manifest: serde_json::Value =
        serde_json::from_slice(&manifest_bytes).expect("manifest json");
    let expected = expected_control_case_count(&manifest);
    assert!(
        expected >= 41,
        "manifest lists {expected} control cases; the suite expects the ticket-05 set"
    );

    let cases = fixtures.control_cases().expect("fixture cases parse");
    assert_eq!(
        cases.len(),
        expected,
        "fixture directory case count must match manifest.json"
    );

    let mut report = ReplayReport::default();
    for (relative, case) in &cases {
        replay_case(relative, case, &mut report);
    }

    let summary = format!(
        "{} cases, {} checks, {} failures\nfailures: {:?}\nskipped: {:?}",
        report.cases.len(),
        report.check_count(),
        report.failures.len(),
        report.failures,
        report.skipped,
    );
    assert!(report.is_ok(), "golden-vector replay FAILED\n{summary}");
    assert!(
        report.skipped.is_empty(),
        "control-plane replay skipped cases — media steps belong in a media harness\n{summary}"
    );
    // Every case must have produced at least one check.
    for case in &report.cases {
        assert!(
            !case.checks.is_empty(),
            "case {} recorded no checks — the harness did not actually verify it",
            case.id
        );
    }
}

#[test]
fn replay_covers_every_control_category() {
    let fixtures = FixtureSet::repo_default().expect("fixtures present");
    let cases = fixtures.control_cases().expect("cases parse");
    let mut report = ReplayReport::default();
    for (relative, case) in &cases {
        replay_case(relative, case, &mut report);
    }
    let ids: Vec<&str> = report.cases.iter().map(|c| c.id.as_str()).collect();
    // One representative per §6 band and per category (manifest's coverage map).
    for probe in [
        "control.auth-happy-path",
        "control.pairing-happy-path-qr",
        "control.pairing-happy-path-sas",
        "control.negotiate-happy-path",
        "control.subscribe-config-diagnostics",
        "control.error-101-version-mismatch",
        "control.error-201-pairing-declined",
        "control.error-202-authorization-unknown",
        "control.error-203-authentication-failed",
        "control.error-204-authorization-denied",
        "control.error-301-busy-transport-close",
        "control.error-302-rate-limited",
        "control.error-303-session-superseded",
        "control.error-401-unset-body",
        "control.error-401-undecodable",
        "control.error-401-frame-length-zero",
        "control.error-401-frame-length-oversize",
        "control.error-402-duplicate-subscribe-nonfatal",
        "control.error-403-unsupported-media-configuration",
        "control.error-501-capture-unhealthy",
        "control.error-502-stream-ended-capture-end",
        "control.error-599-internal-error-detail-bound",
        "control.error-602-datagram-unsupported",
        "control.error-701-client-stopping",
        "limits.frame-cap-65536-accepted",
        "limits.preauth-response-128b",
        "limits.pairing-response-320b",
    ] {
        assert!(
            ids.contains(&probe),
            "case {probe} missing from the replay report"
        );
    }
}

#[test]
fn corrupted_signature_cases_fail_verification() {
    // The negative crypto cases (203, 201-proof-invalid) must record the
    // explicit `signature-invalid-confirmed` check — the corrupted signature
    // was offered to the verifier and provably rejected (and the positive
    // cases must never record it).
    let fixtures = FixtureSet::repo_default().expect("fixtures present");
    let cases = fixtures.control_cases().expect("cases parse");
    for (relative, case) in &cases {
        let negative = matches!(
            case.id.as_str(),
            "control.error-203-authentication-failed" | "control.error-201-pairing-proof-invalid"
        );
        if !negative && !case.id.contains("auth-") && !case.id.contains("pairing-") {
            continue;
        }
        let mut report = ReplayReport::default();
        replay_case(relative, case, &mut report);
        let confirmed_invalid = report
            .cases
            .iter()
            .any(|case_report| case_report.checks.contains(&"signature-invalid-confirmed"));
        if negative {
            assert!(
                confirmed_invalid,
                "case {} must confirm its corrupted signature is rejected (checks: {:?}, failures: {:?})",
                case.id, report.cases, report.failures
            );
        } else {
            assert!(
                !confirmed_invalid,
                "positive case {} must NOT record signature-invalid-confirmed (checks: {:?})",
                case.id,
                report
                    .cases
                    .iter()
                    .flat_map(|c| c.checks.iter())
                    .collect::<Vec<_>>()
            );
        }
    }
}
