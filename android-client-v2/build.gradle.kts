// wire-protocol-v2 Android Client (ticket 08) — root build file.
// Independent gradle root: android-app/ (the released v0.3.4 line) stays
// untouched — see the directory decision in the ticket's Comments.
plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.protobuf) apply false
}
