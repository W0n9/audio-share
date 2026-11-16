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
).orElse(
    // IDE/Gradle sync may not inherit ANDROID_HOME; use the conventional
    // per-user SDK location as a last resort.
    providers.environmentVariable("HOME").map { "$it/Android/Sdk" }
).map { "$it/ndk/27.2.12479018" }

// Real-device scope stays arm64-v8a by default. `-PandroidAbi=x86_64` selects
// the Android emulator ABI for development/CI only — emulator results are not
// real-device acceptance evidence (roadmap §7.3).
val androidAbi = providers.gradleProperty("androidAbi").orElse("arm64-v8a").get()
val cargoTarget = when (androidAbi) {
    "arm64-v8a" -> "aarch64-linux-android"
    "x86_64" -> "x86_64-linux-android"
    else -> throw GradleException("Unsupported -PandroidAbi=$androidAbi (use arm64-v8a or x86_64)")
}

// TEST-ONLY. The fixture client identity key is compiled into the native
// library only when `-PtestSupport=true` is passed explicitly. Default
// release builds carry no test identity/signing path (production identities
// come from v2 pairing). E2E builds: ./gradlew assembleRelease
// -PtestSupport=true
val testSupport = providers.gradleProperty("testSupport")
    .map(String::toBoolean)
    .getOrElse(false)

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
        testInstrumentationRunner = "dev.audioshare.v2.ControlSessionInstrumentation"
        ndk {
            // arm64-v8a is the real-device scope; x86_64 is the emulator ABI.
            abiFilters += androidAbi
        }
        // TEST_SUPPORT gates every test-only identity/addressing code path
        // (adb extras and the native fixture identity). Production releases
        // are built without -PtestSupport and have this false.
        buildConfigField("boolean", "TEST_SUPPORT", testSupport.toString())
    }

    buildFeatures {
        buildConfig = true
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
            // The test-support harness is reached by Class.forName; keep its
            // name only in explicitly flagged builds.
            if (testSupport) {
                proguardFiles("proguard-test-support.pro")
            }
            // 测试 APK 是独立的 API 使用者，R8 必须保留它消费的本地 Binder seam。
            if (providers.gradleProperty("deviceTests").map(String::toBoolean).getOrElse(false)) {
                proguardFiles("proguard-device-tests.pro")
            }
        }
    }

    // 设备回归必须覆盖经过 R8 压缩的真实 release APK。
    testBuildType = "release"

    kotlin {
        jvmToolchain(17)
    }

    sourceSets {
        getByName("main") {
            // generated dir under build/ so `clean` starts without a stale
            // native lib in source dirs
            // Keep the ABI subdirectory layout (<dir>/<abi>/lib*.so); abiFilters
            // selects which ABI is packaged.
            jniLibs.srcDir("build/generated/jniLibs")
            // ONE schema, two ends: the Kotlin codegen compiles the same
            // frozen .proto the Rust side does (never a copy).
            proto {
                srcDir("$rootDir/../protos/wire-protocol-v2")
            }
            // TEST-SUPPORT ONLY: the adb-extras lifecycle harness. Without
            // -PtestSupport the class is absent from the source set, so a
            // production build has no extras-addressing path at all.
            if (testSupport) {
                java.srcDir("src/testSupport/java")
                kotlin.srcDir("src/testSupport/java")
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
    testImplementation("io.mockk:mockk:1.13.16")
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

val nativeDir = rootProject.projectDir.resolve("native")
val cargoOut = nativeDir.resolve("target/${cargoTarget}/release")
val jniLibsDir = projectDir.resolve("build/generated/jniLibs/$androidAbi")

val buildNative by tasks.registering(Exec::class) {
    onlyIf { !project.hasProperty("noNative") }
    workingDir = nativeDir
    environment("ANDROID_HOME", providers.environmentVariable("ANDROID_HOME").orElse("").get())
    val cargoArgs = mutableListOf(
        "cargo", "build", "--release", "--target", cargoTarget,
    )
    if (testSupport) {
        cargoArgs += listOf("--features", "test-support")
    }
    commandLine(cargoArgs)
    inputs.property("testSupport", testSupport)
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
    // Resolve ndkDir at execution time so Gradle sync/configuration does not
    // fail when the SDK environment is absent.
    doFirst {
        commandLine(
            "bash", "-c",
            "mkdir -p ${jniLibsDir.absolutePath} && " +
                "cp ${so.absolutePath} ${dest.absolutePath} && " +
                "${ndkDir.get()}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip " +
                "${dest.absolutePath}"
        )
    }
}

tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }.configureEach {
    dependsOn(stripNative)
}
