package dev.audioshare.v2

import audio.share.v2.Control
import com.google.protobuf.ByteString
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Golden-vector replay harness for `fixtures/wire-protocol-v2/` (README
 * contract), Kotlin side. The Android Client replays the SAME files the Rust
 * side does — never the Rust encoder's output. Per the README's encoding
 * policy: build each `frame` step's message from `decoded` with
 * protobuf-kotlin-lite (java-lite generated code lives inside
 * `audio.share.v2.Control`), prepend the u32 BE length, and require byte
 * equality with `frame_hex` / `body_hex`; parse `body_hex` back and require
 * the re-encode to round-trip; check `size_bounds`.
 *
 * Steps whose `decoded` is not a plain message (unknown-field shapes, tag-0x0a
 * deliberate garbage) are decode-direction only — same carve-out as the Rust
 * harness documents.
 */
object Fixtures {
    /** Repo-root fixtures dir, resolved from any working directory. */
    val FIXTURE_ROOT: File = sequenceOf(
        File("../fixtures/wire-protocol-v2"),
        File("../../fixtures/wire-protocol-v2"),
        File("fixtures/wire-protocol-v2"),
    ).first { it.isDirectory }

    class Step(
        val kind: String,
        val dir: String,
        val message: String,
        val frameHex: String,
        val bodyHex: String,
        val decoded: JSONObject,
        val hasUnknownFields: Boolean,
        val closeCode: Long,
    ) {
        val isFrame: Boolean get() = kind == "frame"
        val isRawStream: Boolean get() = kind == "raw-stream"
        val isTransportClose: Boolean get() = kind == "transport-close"
    }

    class SizeBound(val frame: String, val maxFrameBytes: Int)

    class Case(
        val id: String,
        val preconditions: List<String>,
        val steps: List<Step>,
        val sizeBounds: List<SizeBound>,
    )

    private fun JSONArray.jsonObjects(): List<JSONObject> =
        (0 until length()).map { i -> getJSONObject(i) }

    private fun JSONArray.strings(): List<String> =
        (0 until length()).map { i -> getString(i) }

    fun load(rel: String): Case {
        val v = JSONObject(File(FIXTURE_ROOT, rel).readText())
        val steps = v.getJSONArray("steps").jsonObjects().map { s ->
            Step(
                kind = s.getString("kind"),
                dir = s.optString("dir", ""),
                message = s.optString("message", ""),
                frameHex = s.optString("frame_hex", ""),
                bodyHex = s.optString("body_hex", ""),
                decoded = s.optJSONObject("decoded") ?: JSONObject(),
                hasUnknownFields = s.optBoolean("has_unknown_fields", false),
                closeCode = s.optLong("code", 0),
            )
        }
        val bounds = v.optJSONArray("size_bounds")?.jsonObjects()?.map {
            SizeBound(it.getString("frame"), it.getInt("max_frame_bytes"))
        } ?: emptyList()
        return Case(
            id = v.getString("id"),
            preconditions = v.optJSONArray("preconditions")?.strings() ?: emptyList(),
            steps = steps,
            sizeBounds = bounds,
        )
    }

    /** Every control + limits case, in manifest order (drift guard). */
    fun manifestControlCases(): List<String> {
        val m = JSONObject(File(FIXTURE_ROOT, "manifest.json").readText())
        return m.getJSONArray("cases").jsonObjects()
            .map { it.getString("file") }
            .filter { it.startsWith("control/") || it.startsWith("limits/") }
    }

    // ---- hex helpers ---------------------------------------------------------

    fun unhex(s: String): ByteArray {
        val out = ByteArray(s.length / 2)
        for (i in out.indices) {
            out[i] = s.substring(2 * i, 2 * i + 2).toInt(16).toByte()
        }
        return out
    }

    fun hex(b: ByteArray): String = b.joinToString("") { "%02x".format(it) }

    private fun jbytes(v: JSONObject, key: String): ByteString =
        ByteString.copyFrom(unhex(v.getString(key)))

    private fun jbytesOpt(v: JSONObject, key: String): ByteString? =
        if (v.has(key) && !v.isNull(key)) jbytes(v, key) else null

    private fun jstr(v: JSONObject, key: String): String = v.optString(key, "")

    private fun jstatus(v: JSONObject): Control.Status? {
        if (!v.has("status")) return null
        val s = v.getJSONObject("status")
        val b = Control.Status.newBuilder()
        if (s.has("code")) b.code = s.getLong("code").toInt()
        if (s.optBoolean("retryable", false)) b.retryable = true
        if (s.optBoolean("requires_repairing", false)) b.requiresRepairing = true
        if (s.has("detail")) b.detail = s.getString("detail")
        return b.build()
    }

    private fun jconfiguration(v: JSONObject): Control.Configuration {
        val b = Control.Configuration.newBuilder()
        if (v.has("config_generation")) {
            b.configGeneration = v.getLong("config_generation").toInt()
        }
        if (v.has("codec")) b.codecValue = v.getLong("codec").toInt()
        if (v.has("pcm")) {
            val p = v.getJSONObject("pcm")
            val pb = Control.PcmParameters.newBuilder()
            if (p.has("sample_rate")) pb.sampleRate = p.getLong("sample_rate").toInt()
            if (p.has("channels")) pb.channels = p.getLong("channels").toInt()
            if (p.has("sample_format")) pb.sampleFormatValue = p.getLong("sample_format").toInt()
            b.pcm = pb.build()
        }
        return b.build()
    }

    private fun jcapabilities(v: JSONObject, key: String): List<Int> {
        val a = v.optJSONArray(key) ?: return emptyList()
        return (0 until a.length()).map { i -> a.getInt(i) }
    }

    /**
     * Build the Frame for one `frame` step from its `decoded` content — the
     * Kotlin encoder direction of the README contract. Returns null for steps
     * that are not a plain message (unknown-field shapes); the caller treats
     * those as decode-only.
     */
    fun buildFrameFromDecoded(decoded: JSONObject, message: String): Control.Frame? {
        val b = Control.Frame.newBuilder()
        when (message) {
            "auth_init" -> b.authInit = Control.AuthInit.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .setClientPublicKey(jbytes(decoded, "client_public_key"))
                .build()
            "auth_challenge" -> b.authChallenge = Control.AuthChallenge.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .setServerRandom(jbytes(decoded, "server_random"))
                .build()
            "auth_proof" -> b.authProof = Control.AuthProof.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .setSignature(jbytes(decoded, "signature"))
                .build()
            "auth_result" -> b.authResult = Control.AuthResult.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .also { jstatus(decoded)?.let { s -> it.status = s } }
                .setSessionId(decoded.optLong("session_id", 0))
                .build()
            "negotiate" -> b.negotiate = Control.Negotiate.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .setProtocolMajor(decoded.optLong("protocol_major", 0).toInt())
                .setProtocolMinor(decoded.optLong("protocol_minor", 0).toInt())
                .addAllCapabilities(jcapabilities(decoded, "capabilities"))
                .build()
            "negotiate_response" -> b.negotiateResponse = Control.NegotiateResponse.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .also { jstatus(decoded)?.let { s -> it.status = s } }
                .setProtocolMajor(decoded.optLong("protocol_major", 0).toInt())
                .setProtocolMinor(decoded.optLong("protocol_minor", 0).toInt())
                .addAllCapabilities(jcapabilities(decoded, "capabilities"))
                .build()
            "error" -> b.error = Control.Error.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .also { jstatus(decoded)?.let { s -> it.status = s } }
                .build()
            "close" -> b.close = Control.Close.newBuilder()
                .setCode(decoded.optLong("code", 0).toInt())
                .setDetail(jstr(decoded, "detail"))
                .build()
            "pairing_request" -> b.pairingRequest = Control.PairingRequest.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .setPairingModeValue(decoded.optLong("pairing_mode", 0).toInt())
                .also { jbytesOpt(decoded, "pairing_nonce")?.let { x -> it.pairingNonce = x } }
                .setClientRandom(jbytes(decoded, "client_random"))
                .setClientPublicKey(jbytes(decoded, "client_public_key"))
                .setProtocolMajor(decoded.optLong("protocol_major", 0).toInt())
                .addAllCapabilities(jcapabilities(decoded, "capabilities"))
                .setClientName(jstr(decoded, "client_name"))
                .build()
            "pairing_response" -> b.pairingResponse = Control.PairingResponse.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .also { jstatus(decoded)?.let { s -> it.status = s } }
                .setPairingModeValue(decoded.optLong("pairing_mode", 0).toInt())
                .also { jbytesOpt(decoded, "pairing_nonce")?.let { x -> it.pairingNonce = x } }
                .setClientRandom(jbytes(decoded, "client_random"))
                .setServerRandom(jbytes(decoded, "server_random"))
                .setServerPublicKey(jbytes(decoded, "server_public_key"))
                .setProtocolMajor(decoded.optLong("protocol_major", 0).toInt())
                .addAllCapabilities(jcapabilities(decoded, "capabilities"))
                .setBinding(jbytes(decoded, "binding"))
                .setClientId(jbytes(decoded, "client_id"))
                .setDisplayName(jstr(decoded, "display_name"))
                .build()
            "pairing_confirm" -> b.pairingConfirm = Control.PairingConfirm.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .setBindingEcho(jbytes(decoded, "binding_echo"))
                .setClientProof(jbytes(decoded, "client_proof"))
                .build()
            "pairing_result" -> b.pairingResult = Control.PairingResult.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .also { jstatus(decoded)?.let { s -> it.status = s } }
                .build()
            "subscribe" -> b.subscribe = Control.Subscribe.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .build()
            "subscribe_response" -> b.subscribeResponse = Control.SubscribeResponse.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .also { jstatus(decoded)?.let { s -> it.status = s } }
                .also {
                    if (decoded.has("configuration")) {
                        it.configuration = jconfiguration(decoded.getJSONObject("configuration"))
                    }
                }
                .build()
            "unsubscribe" -> b.unsubscribe = Control.Unsubscribe.newBuilder()
                .setRequestId(decoded.getLong("request_id"))
                .build()
            "unsubscribe_response" -> b.unsubscribeResponse =
                Control.UnsubscribeResponse.newBuilder()
                    .setRequestId(decoded.getLong("request_id"))
                    .also { jstatus(decoded)?.let { s -> it.status = s } }
                    .build()
            "request_configuration" -> b.requestConfiguration =
                Control.RequestConfiguration.newBuilder()
                    .setRequestId(decoded.getLong("request_id"))
                    .build()
            "configuration_published" -> b.configurationPublished =
                Control.ConfigurationPublished.newBuilder()
                    .setRequestId(decoded.getLong("request_id"))
                    .also {
                        if (decoded.has("configuration")) {
                            it.configuration = jconfiguration(decoded.getJSONObject("configuration"))
                        }
                    }
                    .build()
            "stream_started" -> b.streamStarted = Control.StreamStarted.newBuilder()
                .setStreamInstanceId(decoded.optLong("stream_instance_id", 0).toInt())
                .setConfigGeneration(decoded.optLong("config_generation", 0).toInt())
                .setInitialTimestamp(decoded.optLong("initial_timestamp", 0))
                .build()
            "stream_ended" -> b.streamEnded = Control.StreamEnded.newBuilder()
                .setStreamInstanceId(decoded.optLong("stream_instance_id", 0).toInt())
                .setCode(decoded.optLong("code", 0).toInt())
                .setDetail(jstr(decoded, "detail"))
                .build()
            "playback_diagnostics" -> b.playbackDiagnostics = buildDiagnostics(decoded)
            else -> return null
        }
        return b.build()
    }

    private fun buildDiagnostics(v: JSONObject): Control.PlaybackDiagnostics {
        val b = Control.PlaybackDiagnostics.newBuilder()
        fun u32(key: String, set: (Control.PlaybackDiagnostics.Builder, Int) -> Unit) {
            if (v.has(key)) {
                val x = v.getLong(key)
                if (x != 0L) set(b, x.toInt())
            }
        }
        fun counter(key: String, set: (Control.PlaybackDiagnostics.Builder, Long) -> Unit) {
            if (v.has(key)) {
                val x = v.getLong(key)
                if (x != 0L) set(b, x)
            }
        }
        u32("stream_instance_id") { x, n -> x.streamInstanceId = n }
        u32("config_generation") { x, n -> x.configGeneration = n }
        u32("max_reorder_distance") { x, n -> x.maxReorderDistance = n }
        u32("jitter_microseconds") { x, n -> x.jitterMicroseconds = n }
        u32("buffer_current_ms") { x, n -> x.bufferCurrentMs = n }
        u32("buffer_target_ms") { x, n -> x.bufferTargetMs = n }
        counter("extended_highest_sequence") { x, n -> x.extendedHighestSequence = n }
        counter("cumulative_lost") { x, n -> x.cumulativeLost = n }
        counter("interval_lost") { x, n -> x.intervalLost = n }
        counter("cumulative_received") { x, n -> x.cumulativeReceived = n }
        counter("interval_received") { x, n -> x.intervalReceived = n }
        counter("cumulative_duplicates") { x, n -> x.cumulativeDuplicates = n }
        counter("interval_duplicates") { x, n -> x.intervalDuplicates = n }
        counter("cumulative_reordered") { x, n -> x.cumulativeReordered = n }
        counter("interval_reordered") { x, n -> x.intervalReordered = n }
        counter("cumulative_late_drops") { x, n -> x.cumulativeLateDrops = n }
        counter("interval_late_drops") { x, n -> x.intervalLateDrops = n }
        counter("cumulative_unknown_config_drops") { x, n -> x.cumulativeUnknownConfigDrops = n }
        counter("interval_unknown_config_drops") { x, n -> x.intervalUnknownConfigDrops = n }
        counter("cumulative_concealments") { x, n -> x.cumulativeConcealments = n }
        counter("interval_concealments") { x, n -> x.intervalConcealments = n }
        counter("concealed_frames") { x, n -> x.concealedFrames = n }
        counter("underruns") { x, n -> x.underruns = n }
        counter("submitted_position") { x, n -> x.submittedPosition = n }
        return b.build()
    }

    // ---- replay checks -------------------------------------------------------

    /** u32 BE length | Frame body — spec §3. */
    fun framed(frame: Control.Frame): ByteArray {
        val body = frame.toByteArray()
        val out = ByteArray(4 + body.size)
        out[0] = (body.size ushr 24).toByte()
        out[1] = (body.size ushr 16).toByte()
        out[2] = (body.size ushr 8).toByte()
        out[3] = body.size.toByte()
        body.copyInto(out, 4)
        return out
    }

    /** README contract item 2: encoding direction, byte equality. */
    fun assertEncodingMatches(step: Step) {
        val frame = buildFrameFromDecoded(step.decoded, step.message)
            ?: return // decode-only shapes
        val framedBytes = framed(frame)
        kotlin.test.assertEquals(step.frameHex, hex(framedBytes), "framed (${step.message})")
        kotlin.test.assertEquals(step.bodyHex, hex(frame.toByteArray()), "body (${step.message})")
    }

    /**
     * README contract item 3: decoding direction. Parse body_hex; the result
     * must round-trip byte-exact. has_unknown_fields steps are decode-only:
     * every conforming proto3 runtime drops unknown fields on re-encode (the
     * spec's documented ignore rule, §4.4). Bodies that deliberately do not
     * parse (fixture error-401-undecodable) are asserted undecodable instead —
     * the 401 outcome is pinned by that case's own replay test.
     */
    fun assertDecodingRoundtrip(step: Step) {
        val body = unhex(step.bodyHex)
        val frame = try {
            Control.Frame.parseFrom(body)
        } catch (e: Exception) {
            if (step.message == "<undecodable>") return // asserted separately
            throw AssertionError("decode failed for ${step.message}: $e")
        }
        if (!step.hasUnknownFields) {
            kotlin.test.assertEquals(step.bodyHex, hex(frame.toByteArray()), "roundtrip")
            val prefix = unhex(step.frameHex)
            val len = ((prefix[0].toInt() and 0xff) shl 24) or
                ((prefix[1].toInt() and 0xff) shl 16) or
                ((prefix[2].toInt() and 0xff) shl 8) or
                (prefix[3].toInt() and 0xff)
            kotlin.test.assertEquals(body.size, len, "length prefix")
        }
    }

    /** README contract item 6: size_bounds checked during replay. */
    fun assertSizeBounds(case: Case) {
        for (step in case.steps) {
            if (!step.isFrame) continue
            for (bound in case.sizeBounds) {
                if (step.message.contains(bound.frame)) {
                    val framedLen = unhex(step.frameHex).size
                    kotlin.test.assertTrue(
                        framedLen <= bound.maxFrameBytes,
                        "${step.message} exceeds ${bound.maxFrameBytes} B bound: $framedLen",
                    )
                }
            }
        }
    }

    /** Full replay of one case: both directions + bounds. */
    fun assertCase(rel: String) {
        val case = load(rel)
        for (step in case.steps) {
            if (!step.isFrame) continue
            if (!step.hasUnknownFields) assertEncodingMatches(step)
            assertDecodingRoundtrip(step)
        }
        assertSizeBounds(case)
    }

    /**
     * The deliberately-undecodable body of error-401-undecodable (tag 0x0a,
     * length varint 255, one byte follows) must fail proto parsing — the
     * Client answers 401.
     */
    fun assertUndecodable(bodyHex: String) {
        try {
            Control.Frame.parseFrom(unhex(bodyHex))
            throw AssertionError("body must not parse: $bodyHex")
        } catch (_: Exception) {
            // expected
        }
    }
}
