package dev.audioshare.v2;

import android.app.Instrumentation;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.ServiceConnection;
import android.os.Bundle;
import android.os.IBinder;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.security.AlgorithmParameters;
import java.security.KeyFactory;
import java.security.KeyStore;
import java.security.PrivateKey;
import java.security.PublicKey;
import java.security.Signature;
import java.security.spec.ECGenParameterSpec;
import java.security.spec.ECParameterSpec;
import java.security.spec.ECPoint;
import java.security.spec.ECPublicKeySpec;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Ticket 20 device contract: Keystore identity, Service/Binder pairing, and
 * credential reload. Driven through the existing custom runner:
 *
 * <pre>
 * adb shell am instrument -w -r -e case keystore \
 *   dev.audioshare.v2.test/dev.audioshare.v2.ControlSessionInstrumentation
 * </pre>
 *
 * Cases:
 * - `keystore`  — generate/sign/verify a non-exportable Keystore identity.
 * - `pairSas`   — pair via the Service using `-e address host:port`.
 * - `pairQr`    — pair via the Service using `-e uri audioshare-pair://...`.
 * - `reload`    — connect using the stored credential, expect READY/PLAYING.
 * - `revoked`   — connect stored credential, expect typed 204 guidance.
 * - `unknown`   — connect stored credential, expect typed 202 guidance.
 */
final class PairingContract {
    private PairingContract() {}

    private static SessionController lastController;

    static boolean isPairingCase(String testCase) {
        return "keystore".equals(testCase) || "pairSas".equals(testCase)
            || "pairQr".equals(testCase) || "reload".equals(testCase)
            || "revoked".equals(testCase) || "unknown".equals(testCase)
            || "identityChange".equals(testCase)
            || "harness".equals(testCase);
    }

    static void run(Instrumentation instrumentation, Bundle arguments, String testCase) throws Exception {
        switch (testCase) {
            case "keystore":
                runKeystoreCase(instrumentation);
                break;
            case "pairSas":
                runPairingCase(instrumentation, arguments.getString("address"), null);
                break;
            case "pairQr":
                runPairingCase(instrumentation, null, arguments.getString("uri"));
                break;
            case "reload":
                runStoredConnectCase(instrumentation, null, null);
                break;
            case "revoked":
                runStoredConnectCase(instrumentation, 204, arguments.getString("pin"));
                break;
            case "unknown":
                runStoredConnectCase(instrumentation, 202, arguments.getString("pin"));
                break;
            case "identityChange":
                runStoredConnectCase(instrumentation, 205, arguments.getString("pin"));
                break;
            case "harness":
                runHarnessCase(instrumentation);
                break;
            default:
                throw new AssertionError("unknown pairing case " + testCase);
        }
    }

    private static void runKeystoreCase(Instrumentation instrumentation) throws Exception {
        Context context = instrumentation.getTargetContext();
        String alias = "instrumentation-keystore";
        ClientIdentityResult identity = ClientIdentity.INSTANCE.create(context, alias);
        if (identity.getSoftwareBacked()) {
            throw new AssertionError("Android Keystore must be available on the test device");
        }
        byte[] raw = "instrumentation raw transcript".getBytes(StandardCharsets.UTF_8);
        byte[] signature = identity.getSigner().sign(raw);
        if (signature.length != 64) {
            throw new AssertionError("ClientSigner.sign must return 64-byte r||s, got " + signature.length);
        }
        verifySec1(identity.getSigner().publicKeySec1(), raw, signature);
        // The Keystore private key must be non-exportable.
        KeyStore keyStore = KeyStore.getInstance("AndroidKeyStore");
        keyStore.load(null);
        PrivateKey privateKey = (PrivateKey) keyStore.getKey(alias, null);
        if (privateKey == null) throw new AssertionError("Keystore key missing");
        if (privateKey.getEncoded() != null) {
            throw new AssertionError("Keystore private key must be non-exportable");
        }
        sendProgress(instrumentation, "KEYSTORE_OK hardware_backed=" + identity.getHardwareBacked());
    }

    private static void runPairingCase(Instrumentation instrumentation, String address, String uri)
        throws Exception {
        SessionController service = bindController(instrumentation);
        if (uri != null) {
            service.beginQrPairing(uri);
        } else {
            if (address == null || address.isEmpty()) throw new AssertionError("address is required");
            service.beginSasPairing(address);
        }
        PairingSnapshot awaiting = awaitPairingPhase("AWAITING_CONFIRMATION", 20_000);
        if (awaiting.getSas() == null) throw new AssertionError("pairing code missing");
        if (awaiting.getFingerprint() == null) throw new AssertionError("fingerprint missing");
        sendProgress(instrumentation, "PAIRING_CODE " + awaiting.getSas());
        service.confirmPairing();
        awaitPairingPhase("COMPLETE", 20_000);
        // `as-cmd pair` deliberately exits after persisting, so the paired
        // connection is torn down; the credential must still be captured.
        awaitCredential(service, 10_000);
        List<CredentialRecord> records = service.credentials();
        if (records.isEmpty()) throw new AssertionError("pairing credential was not persisted");
        CredentialRecord record = records.get(records.size() - 1);
        if (record.getKeyAlias().isEmpty()) throw new AssertionError("credential has no key alias");
        // A newly bound UI (Activity/window rebuild) must see the same state.
        PairingSnapshot replay = bindController(instrumentation).pairingSnapshot();
        if (replay == null || !"COMPLETE".equals(replay.getPhase())) {
            throw new AssertionError("re-bound controller did not replay COMPLETE: " +
                (replay == null ? "<none>" : replay.getPhase()));
        }
        sendProgress(instrumentation, "CREDENTIAL pin=" + record.getServerSpkiSha256Hex().substring(0, 16) +
            " software_backed=" + record.getSoftwareBacked());
        service.disconnect();
    }

    private static void awaitCredential(SessionController service, long timeoutMillis)
        throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis);
        while (System.nanoTime() < deadline) {
            if (!service.credentials().isEmpty()) return;
            Thread.sleep(25);
        }
        throw new AssertionError("PairingResult(OK) did not reach the credential store");
    }

    /**
     * TEST-SUPPORT ONLY: the adb-extras harness is reached reflectively by
     * MainActivity/Service, so verify R8 kept it and it parses the config.
     */
    private static void runHarnessCase(Instrumentation instrumentation) throws Exception {
        Intent intent = new Intent()
            .putExtra("host", "10.0.2.2")
            .putExtra("port", 65532)
            .putExtra("pinnedSpki", "00".repeat(32))
            .putExtra("serverKey", "11".repeat(32))
            .putExtra("clientName", "harness-client");
        Class<?> harness = Class.forName("dev.audioshare.v2.TestSupportHarness");
        Object parsed = harness.getMethod("serviceConfiguration", Intent.class)
            .invoke(null, intent);
        if (parsed == null) throw new AssertionError("test-support config was not parsed");
        SessionConfiguration configuration = (SessionConfiguration) parsed;
        if (!configuration.getUseNativeTestIdentity()) {
            throw new AssertionError("test-support config must use the native test identity");
        }
        if (!"10.0.2.2".equals(configuration.getHost()) || configuration.getPort() != 65532) {
            throw new AssertionError("harness config address mismatch");
        }
        Object absent = harness.getMethod("serviceConfiguration", Intent.class)
            .invoke(null, new Intent());
        if (absent != null) throw new AssertionError("empty intent must not yield a config");
        sendProgress(instrumentation, "HARNESS_OK");
    }

    private static void runStoredConnectCase(Instrumentation instrumentation, Integer expectedError, String pin)
        throws Exception {
        SessionController service = bindController(instrumentation);
        service.connectStored(pin);
        if (expectedError == null) {
            awaitLiveState(20_000);
            List<CredentialRecord> records = service.credentials();
            if (records.isEmpty()) throw new AssertionError("no stored credential");
            sendProgress(instrumentation, "RELOAD_OK pin=" + records.get(0).getServerSpkiSha256Hex().substring(0, 16));
            service.disconnect();
            return;
        }
        awaitClosed(20_000);
        ErrorGuidance guidance = service.lastErrorGuidance();
        if (guidance == null) throw new AssertionError("no typed error guidance after close");
        if (guidance.getCode() != expectedError) {
            throw new AssertionError("expected error " + expectedError + ", got " + guidance.getCode());
        }
        if (guidance.getFallbackToV1()) throw new AssertionError("v2 must never fall back to v1");
        sendProgress(instrumentation, "ERROR_OK code=" + guidance.getCode() + " name=" + guidance.getName());
        service.disconnect();
    }

    private static SessionController bindController(Instrumentation instrumentation)
        throws InterruptedException {
        final CountDownLatch bound = new CountDownLatch(1);
        final AtomicReference<SessionController> owner = new AtomicReference<>();
        ServiceConnection connection = new ServiceConnection() {
            @Override public void onServiceConnected(ComponentName name, IBinder binder) {
                owner.set(((ControlBinder) binder).getService());
                bound.countDown();
            }
            @Override public void onServiceDisconnected(ComponentName name) {}
        };
        Intent intent = new Intent().setClassName(instrumentation.getTargetContext(),
            "dev.audioshare.v2.ControlSessionService");
        if (!instrumentation.getTargetContext().bindService(intent, connection, Context.BIND_AUTO_CREATE)) {
            throw new AssertionError("cannot bind ControlSessionService");
        }
        if (!bound.await(5, TimeUnit.SECONDS)) throw new AssertionError("no Service binding");
        lastController = owner.get();
        return lastController;
    }

    private static PairingSnapshot awaitPairingPhase(String phase, long timeoutMillis)
        throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis);
        PairingSnapshot snapshot = null;
        while (System.nanoTime() < deadline) {
            snapshot = lastController == null ? null : lastController.pairingSnapshot();
            if (snapshot != null && phase.equals(snapshot.getPhase())) return snapshot;
            if (snapshot != null && "FAILED".equals(snapshot.getPhase())) {
                throw new AssertionError("pairing failed with code " + snapshot.getErrorCode() +
                    " (sas=" + snapshot.getSas() + ")");
            }
            Thread.sleep(50);
        }
        throw new AssertionError("pairing did not reach " + phase +
            "; last=" + (snapshot == null ? "<none>" : snapshot.getPhase()));
    }

    private static void awaitLiveState(long timeoutMillis) throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis);
        while (System.nanoTime() < deadline) {
            String state = lastController == null ? null : lastController.stateSnapshot();
            if ("READY".equals(state) || "PLAYING".equals(state)) return;
            Thread.sleep(50);
        }
        throw new AssertionError("session did not reach READY/PLAYING; last=" +
            (lastController == null ? "<none>" : lastController.stateSnapshot()));
    }

    private static void awaitClosed(long timeoutMillis) throws InterruptedException {
        long deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMillis);
        while (System.nanoTime() < deadline) {
            String state = lastController == null ? null : lastController.stateSnapshot();
            if ("CLOSED".equals(state)) return;
            Thread.sleep(50);
        }
        throw new AssertionError("session did not close; last=" +
            (lastController == null ? "<none>" : lastController.stateSnapshot()));
    }

    /** Rebuild the SEC1 point and verify the raw-context signature with JCA. */
    private static void verifySec1(byte[] sec1, byte[] raw, byte[] signature) throws Exception {
        if (sec1.length != 33 || (sec1[0] != 0x02 && sec1[0] != 0x03)) {
            throw new AssertionError("signer returned a non-SEC1 public key");
        }
        byte[] x = new byte[32];
        System.arraycopy(sec1, 1, x, 0, 32);
        AlgorithmParameters params = AlgorithmParameters.getInstance("EC");
        params.init(new ECGenParameterSpec("secp256r1"));
        ECParameterSpec spec = params.getParameterSpec(ECParameterSpec.class);
        BigInteger p = ((java.security.spec.ECFieldFp) spec.getCurve().getField()).getP();
        BigInteger a = spec.getCurve().getA();
        BigInteger b = spec.getCurve().getB();
        BigInteger xInt = new BigInteger(1, x);
        BigInteger rhs = xInt.modPow(BigInteger.valueOf(3), p)
            .add(a.multiply(xInt).mod(p))
            .add(b)
            .mod(p);
        BigInteger y = sqrtModP(rhs, p);
        boolean odd = (sec1[0] == 0x03);
        if (y.testBit(0) != odd) y = p.subtract(y);
        ECPublicKeySpec keySpec = new ECPublicKeySpec(new ECPoint(xInt, y), spec);
        PublicKey publicKey = KeyFactory.getInstance("EC").generatePublic(keySpec);

        Signature verifier = Signature.getInstance("SHA256withECDSA");
        verifier.initVerify(publicKey);
        verifier.update(raw);
        if (!verifier.verify(DerSignature.INSTANCE.toDer(signature))) {
            throw new AssertionError("Keystore signature does not verify over the raw context");
        }
    }

    /** P-256: p ≡ 3 (mod 4), so sqrt = value^((p+1)/4). */
    private static BigInteger sqrtModP(BigInteger value, BigInteger p) {
        return value.modPow(p.add(BigInteger.ONE).shiftRight(2), p);
    }

    private static void sendProgress(Instrumentation instrumentation, String message) {
        Bundle status = new Bundle();
        status.putString("stream", message + "\n");
        instrumentation.sendStatus(0, status);
    }
}
