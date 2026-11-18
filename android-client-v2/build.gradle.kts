// wire-protocol-v2 Android Client (ticket 08) — root build file.
// Independent gradle root: android-app/ (the released v0.3.4 line) stays
// untouched — see the directory decision in the ticket's Comments.
// AGP 9 bundles KGP 2.2.10 with built-in Kotlin. This project targets KGP
// 2.4.21 (the same version as the Compose compiler plugin), so override the
// bundled version with the documented buildscript classpath.
buildscript {
    repositories {
        google()
        mavenCentral()
    }
    dependencies {
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.21")
    }
}

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.compose) apply false
    alias(libs.plugins.protobuf) apply false
}
