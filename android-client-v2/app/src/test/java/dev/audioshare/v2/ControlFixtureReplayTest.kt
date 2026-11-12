package dev.audioshare.v2

import audio.share.v2.Control
import org.junit.Test
import kotlin.test.assertTrue

/**
 * Golden-vector replay, Kotlin side (fixtures README contract): the SAME
 * fixture files the native Rust tests replay, encoded and decoded here with
 * protobuf-kotlin-lite from the same frozen schema — byte equality in both
 * directions, size bounds, and the negative framing cases.
 *
 * The Kotlin code is never the Rust encoder's consumer: this suite is the
 * independent second half of the cross-language vector contract (README:
 * "any implementation replays the same files").
 */
class ControlFixtureReplayTest {

    private val happyPaths = listOf(
        "control/auth-happy-path.json",
        "control/pairing-happy-path-qr.json",
        "control/pairing-happy-path-sas.json",
        "control/negotiate-happy-path.json",
        "control/negotiate-unknown-capability-ignored.json",
        "control/subscribe-config-diagnostics.json",
        "control/unknown-field-ignored.json",
    )

    private val errorPaths = listOf(
        "control/unknown-oneof-case-401.json",
        "control/subscribe-before-ready-402.json",
        "control/auth-in-wrong-state-402.json",
        "control/connecting-valid-subscribe-402.json",
        "control/malformed-capability-list-negotiate-401.json",
        "control/error-101-version-mismatch.json",
        "control/error-201-pairing-proof-invalid.json",
        "control/error-201-pairing-declined.json",
        "control/error-202-authorization-unknown.json",
        "control/error-203-authentication-failed.json",
        "control/error-204-authorization-denied.json",
        "control/error-204-revoked-active-session.json",
        "control/error-301-busy-transport-close.json",
        "control/error-302-rate-limited.json",
        "control/error-303-session-superseded.json",
        "control/error-401-frame-length-zero.json",
        "control/error-401-frame-length-oversize.json",
        "control/error-401-unset-body.json",
        "control/error-401-undecodable.json",
        "control/error-401-pairing-duplicate-capabilities.json",
        "control/error-401-pairing-capabilities-over-255.json",
        "control/error-401-invalid-configuration.json",
        "control/error-402-duplicate-subscribe-nonfatal.json",
        "control/error-403-unsupported-media-configuration.json",
        "control/error-501-capture-unhealthy.json",
        "control/error-501-capture-failure-stream-ended.json",
        "control/error-502-stream-ended-capture-end.json",
        "control/error-599-internal-error-detail-bound.json",
        "control/error-601-idle-timeout-transport-close.json",
        "control/error-602-datagram-unsupported.json",
        "control/error-701-client-stopping.json",
    )

    private val limitsPaths = listOf(
        "limits/frame-cap-65536-accepted.json",
        "limits/preauth-response-128b.json",
        "limits/pairing-response-320b.json",
    )

    @Test
    fun replay_happy_paths_both_directions() {
        for (rel in happyPaths) Fixtures.assertCase(rel)
    }

    @Test
    fun replay_error_registry_both_directions() {
        for (rel in errorPaths) Fixtures.assertCase(rel)
    }

    @Test
    fun replay_limits_both_directions() {
        for (rel in limitsPaths) Fixtures.assertCase(rel)
    }

    /** Every fixture case is covered by one of the three lists (drift guard). */
    @Test
    fun manifest_case_list_in_sync() {
        val covered = happyPaths + errorPaths + limitsPaths
        val manifest = Fixtures.manifestControlCases()
        assertTrue(
            manifest.size == covered.size,
            "manifest (${manifest.size}) vs runner (${covered.size}) drift",
        )
        for (f in manifest) {
            assertTrue(covered.contains(f), "manifest case $f not replayed by the runner")
        }
    }

    /**
     * error-401-undecodable: the deliberately-garbage body (tag 0x0a wire
     * type 2, length varint 255, one byte follows) must fail parsing — the
     * machine answers Error(401)+Close(401) whose bytes are byte-checked by
     * replay_error_registry (those steps are plain frames).
     */
    @Test
    fun undecodable_body_fails_parse() {
        val case = Fixtures.load("control/error-401-undecodable.json")
        val step = case.steps[0]
        assertTrue(step.isFrame)
        Fixtures.assertUndecodable(step.bodyHex)
    }

    /**
     * unknown-oneof-case-401: field 80 (a reserved range) with a
     * length-delim payload parses as a Frame with NO body set (proto3 unknown
     * fields are dropped; the oneof cannot hold unknown cases) — the
     * unset-body rule then yields 401. The S->C error/close bytes are
     * byte-checked by the registry replay.
     */
    @Test
    fun unknown_oneof_case_reads_as_unset_body() {
        val case = Fixtures.load("control/unknown-oneof-case-401.json")
        val step = case.steps[0]
        val frame = Control.Frame.parseFrom(Fixtures.unhex(step.bodyHex))
        assertTrue(frame.bodyCase == Control.Frame.BodyCase.BODY_NOT_SET)
    }

    /**
     * unknown-field-ignored: the unknown field 15 inside Subscribe is dropped
     * by the lite runtime; the Subscribe content survives — the ignore rule
     * of the §4.4 validation table. The has_unknown_fields step is
     * decode-direction only (both runtimes drop unknowns on re-encode).
     */
    @Test
    fun unknown_field_inside_subscribe_is_ignored() {
        val case = Fixtures.load("control/unknown-field-ignored.json")
        val step = case.steps[0]
        val frame = Control.Frame.parseFrom(Fixtures.unhex(step.bodyHex))
        assertTrue(frame.bodyCase == Control.Frame.BodyCase.SUBSCRIBE)
        kotlin.test.assertEquals(1L, frame.subscribe.requestId)
    }

    /** 701 client stop: the C->S Close(701) body is the fixture's bytes. */
    @Test
    fun close_701_body_matches_fixture() {
        val case = Fixtures.load("control/error-701-client-stopping.json")
        val close = Fixtures.buildFrameFromDecoded(
            case.steps[0].decoded, "close",
        )!!
        kotlin.test.assertEquals(case.steps[0].bodyHex, Fixtures.hex(close.toByteArray()))
    }

    /** 403: Client's own Error+Close frames match the fixture C->S bodies. */
    @Test
    fun client_403_error_and_close_match_fixture() {
        val case = Fixtures.load("control/error-403-unsupported-media-configuration.json")
        val err = Fixtures.buildFrameFromDecoded(case.steps[1].decoded, "error")!!
        val close = Fixtures.buildFrameFromDecoded(case.steps[2].decoded, "close")!!
        kotlin.test.assertEquals(case.steps[1].bodyHex, Fixtures.hex(err.toByteArray()))
        kotlin.test.assertEquals(case.steps[2].bodyHex, Fixtures.hex(close.toByteArray()))
    }
}
