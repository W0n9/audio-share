# 仅 -PdeviceTests=true：保留由独立测试 APK 调用的公开 Binder seam。
-keep class dev.audioshare.v2.ControlBinder {
    public dev.audioshare.v2.ControlSessionService getService();
}
-keepclassmembers class dev.audioshare.v2.ControlSessionService {
    public void setOutcomeListener(kotlin.jvm.functions.Function1);
    public dev.audioshare.v2.SessionOutcome outcomeSnapshot();
}
-keep interface kotlin.jvm.functions.Function1 { *; }

# Ticket 15: the device contract observes the outcome seam by name.
-keep class dev.audioshare.v2.SessionOutcome { *; }
-keep class dev.audioshare.v2.SessionOutcome$* { *; }
-keep class dev.audioshare.v2.LivePhase { *; }
-keep class dev.audioshare.v2.EndReason { *; }
-keep class dev.audioshare.v2.EndReason$* { *; }

# Ticket 20: the pairing/Keystore device contract lives in the separate test
# APK and calls these app classes by name. Device-test builds only; production
# releases do not apply this file.
-keep class dev.audioshare.v2.ClientIdentity { *; }
-keep class dev.audioshare.v2.ClientIdentityResult { *; }
-keep interface dev.audioshare.v2.ClientSigner { *; }
-keep class * implements dev.audioshare.v2.ClientSigner { *; }
-keep class dev.audioshare.v2.PairingSnapshot { *; }
-keep class dev.audioshare.v2.PairingStateRelay { *; }
-keep class dev.audioshare.v2.CredentialRecord { *; }
-keep class dev.audioshare.v2.CredentialStore { *; }
-keep class * implements dev.audioshare.v2.CredentialStore { *; }
-keep class dev.audioshare.v2.SharedPreferencesCredentialStore { *; }
-keep class dev.audioshare.v2.ErrorGuidance { *; }
-keep class dev.audioshare.v2.ErrorGuidanceBook { *; }
-keep class dev.audioshare.v2.SessionController { *; }
-keep class dev.audioshare.v2.SessionConfiguration { *; }
-keep class dev.audioshare.v2.PairingStart { *; }
-keep class dev.audioshare.v2.ControlSessionService {
    public <init>();
    public *;
}
-keep class dev.audioshare.v2.PairingPayload { *; }
-keep class dev.audioshare.v2.SasCode { *; }
-keep class dev.audioshare.v2.DerSignature { *; }
-keep class dev.audioshare.v2.EcKeys { *; }
