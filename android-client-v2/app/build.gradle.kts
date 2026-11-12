@file:Suppress("UnstableApiUsage")

import org.gradle.api.tasks.Exec
import com.google.protobuf.gradle.proto

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.protobuf)
}

val ndkDir = providers.environmentVariable("ANDROID_HOME").orElse(
    providers.environmentVariable("ANDROID_NDK_HOME")
).map { "$it/ndk/27.2.12479018" }

android {
    namespace = "dev.audioshare.v2"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.audioshare.v2"
        // ticket 08 gate: Android 8 / API 26 baseline, same as the prototype
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0-v2"
        ndk {
            // arm64 real-device scope for this ticket
            abiFilters += "arm64-v8a"
        }
    }

    buildTypes {
        release {
            // R8 on: proves the ProGuard rules the integration would need.
            // No release keystore in this line: debug-signed release so
            // `assembleRelease` yields an installable APK.
            isMinifyEnabled = true
            isShrinkResources = true
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
            // generated dir under build/ so `clean` starts without a stale
            // native lib in source dirs
            jniLibs.srcDir("build/generated/jniLibs")
            // ONE schema, two ends: the Kotlin codegen compiles the same
            // frozen .proto the Rust side does (never a copy).
            proto {
                srcDir("$rootDir/../protos/wire-protocol-v2")
            }
        }
    }

    packaging {
        resources {
            excludes += "META-INF/*"
        }
    }
}

dependencies {
    implementation(libs.google.protobuf.kotlin.lite)

    // Unit tests: the fixture replay harness. org.json comes from the real
    // artifact (android.jar's stubs throw in unit tests); kotlin("test")
    // provides the kotlin.test assertions.
    testImplementation(libs.junit)
    testImplementation("org.json:json:20240303")
    testImplementation(kotlin("test"))
}

protobuf {
    protoc {
        artifact = "com.google.protobuf:protoc:${libs.versions.protobuf.get()}"
    }
    generateProtoTasks {
        all().forEach { task ->
            task.builtins {
                create("java") {
                    option("lite")
                }
                create("kotlin") {
                    option("lite")
                }
            }
        }
    }
}

// ---- cargo wiring -----------------------------------------------------------
// Same shape as the prototype's: cargo + NDK clang (direct linker config in
// native/.cargo/config.toml — cargo-ndk is NOT used), then NDK llvm-strip into
// jniLibs. Skipped with -PnoNative=true.

val cargoTarget = "aarch64-linux-android"
val nativeDir = rootProject.projectDir.resolve("native")
val cargoOut = nativeDir.resolve("target/${cargoTarget}/release")
val jniLibsDir = projectDir.resolve("build/generated/jniLibs/arm64-v8a")

val buildNative by tasks.registering(Exec::class) {
    onlyIf { !project.hasProperty("noNative") }
    workingDir = nativeDir
    environment("ANDROID_HOME", providers.environmentVariable("ANDROID_HOME").orElse("").get())
    // TEST-ONLY E2E build: `test-support` enables the fixture client identity
    // key in the native crate (production profiles must not enable it; the
    // real identity comes from v2 pairing, phase 6).
    commandLine(
        "cargo", "build", "--release", "--target", cargoTarget,
        "--features", "test-support",
    )
    outputs.file(cargoOut.resolve("libaudio_share_client_v2_native.so"))
    inputs.dir(nativeDir.resolve("src"))
    inputs.file(nativeDir.resolve("Cargo.toml"))
}

val stripNative by tasks.registering(Exec::class) {
    onlyIf { !project.hasProperty("noNative") }
    dependsOn(buildNative)
    val so = cargoOut.resolve("libaudio_share_client_v2_native.so")
    val dest = jniLibsDir.resolve("libaudio_share_client_v2_native.so")
    inputs.file(so)
    outputs.file(dest)
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
