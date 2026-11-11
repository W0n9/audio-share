// Throwaway prototype project for wire-protocol-v2 ticket 01.
// Versions deliberately mirror the production android-app setup (AGP 8.7.3 /
// Kotlin 2.0.21 / Gradle 8.9) so the transport question is answered on the
// toolchain the product actually uses.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.android) apply false
}
