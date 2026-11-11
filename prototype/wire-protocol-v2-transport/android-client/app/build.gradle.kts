@file:Suppress("UnstableApiUsage")

import org.gradle.api.tasks.Exec

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

val ndkDir = providers.environmentVariable("ANDROID_HOME").orElse(
    providers.environmentVariable("ANDROID_NDK_HOME")
).map { "$it/ndk/27.2.12479018" }

android {
    namespace = "dev.prototype.wireproto"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.prototype.wireproto"
        // Prototype gate: Android 8 / API 26 baseline (ticket 01)
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0-proto"
        ndk {
            // prototype targets a real arm64 device only
            abiFilters += "arm64-v8a"
        }
    }

    buildTypes {
        release {
            // R8 on: proves the ProGuard rules the real integration would need
            isMinifyEnabled = true
            isShrinkResources = true
            // no release keystore in the prototype: debug-signed release so
            // `assembleRelease` yields an installable APK
            signingConfig = signingConfigs.getByName("debug")
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    kotlin {
        jvmToolchain(17)
    }

    sourceSets {
        getByName("main") {
            // generated dir under build/ so `clean` (and -PnoNative baseline
            // builds) start without any stale native lib in source dirs
            jniLibs.srcDir("build/generated/jniLibs")
        }
    }

    packaging {
        resources {
            excludes += "META-INF/*"
        }
    }
}

// ---- cargo wiring -----------------------------------------------------------
// Builds the cdylib with cargo + NDK clang (direct linker config in
// native/.cargo/config.toml — cargo-ndk is NOT used), then strips it with the
// NDK's llvm-strip into jniLibs. Skipped entirely with -PnoNative=true, which
// produces the empty-baseline APK for the size comparison.

val cargoTarget = "aarch64-linux-android"
// NOTE: app module script — the crate lives next to app/ under the root project
val nativeDir = rootProject.projectDir.resolve("native")
val cargoOut = nativeDir.resolve("target/${cargoTarget}/release")
val jniLibsDir = projectDir.resolve("build/generated/jniLibs/arm64-v8a")

val buildNative by tasks.registering(Exec::class) {
    onlyIf { !project.hasProperty("noNative") }
    workingDir = nativeDir
    environment("ANDROID_HOME", providers.environmentVariable("ANDROID_HOME").orElse("").get())
    commandLine("cargo", "build", "--release", "--target", cargoTarget)
    outputs.file(cargoOut.resolve("libaudio_share_proto_native.so"))
}

val stripNative by tasks.registering(Exec::class) {
    onlyIf { !project.hasProperty("noNative") }
    dependsOn(buildNative)
    val so = cargoOut.resolve("libaudio_share_proto_native.so")
    val dest = jniLibsDir.resolve("libaudio_share_proto_native.so")
    inputs.file(so)
    outputs.file(dest)
    // copy unstripped, then strip in place with the NDK llvm-strip
    commandLine(
        "bash", "-c",
        "mkdir -p ${jniLibsDir.absolutePath} && " +
            "cp ${so.absolutePath} ${dest.absolutePath} && " +
            "${ndkDir.get()}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip " +
            "${dest.absolutePath}"
    )
}

tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }.configureEach {
    dependsOn(stripNative)
}

// baseline task alias: ./gradlew assembleRelease -PnoNative
tasks.register("printBaselineHint") {
    doLast { println("Use: ./gradlew clean assembleRelease -PnoNative for the empty baseline") }
}
