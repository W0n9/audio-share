@file:Suppress("UnstableApiUsage")

import org.gradle.api.tasks.Exec
import com.google.protobuf.gradle.proto
import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.protobuf)
}

val ndkDir = providers.environmentVariable("ANDROID_HOME").orElse(
    providers.environmentVariable("ANDROID_NDK_HOME")
).orElse(
    // IDE/Gradle sync may not inherit ANDROID_HOME; use the conventional
    // per-user SDK location as a last resort.
    providers.environmentVariable("HOME").map { "$it/Android/Sdk" }
).map { "$it/ndk/27.2.12479018" }

// ABI selection. The production default stays arm64-v8a (real devices).
// The beta release builds ONE universal APK containing both arm64-v8a and
// x86_64 with `-PandroidAbis=arm64-v8a,x86_64`; `-PandroidAbi=x86_64`
// remains supported for emulator development.
val abiToCargoTarget = mapOf(
    "arm64-v8a" to "aarch64-linux-android",
    "x86_64" to "x86_64-linux-android",
)
val abiRequest = providers.gradleProperty("androidAbis")
    .orElse(providers.gradleProperty("androidAbi"))
    .getOrElse("arm64-v8a")
val androidAbis = abiRequest.split(',').map { it.trim() }.filter { it.isNotEmpty() }.distinct()
require(androidAbis.isNotEmpty()) { "No Android ABI selected" }
androidAbis.forEach { abi ->
    require(abiToCargoTarget.containsKey(abi)) {
        "Unsupported ABI '$abi' (use arm64-v8a and/or x86_64)"
    }
}

// ---- release signing --------------------------------------------------------
// Credentials come from the environment or from a repository-external
// properties file; no secret is ever read from the repository.
//
//   AUDIO_SHARE_KEYSTORE_FILE
//   AUDIO_SHARE_KEYSTORE_PASSWORD
//   AUDIO_SHARE_KEY_ALIAS
//   AUDIO_SHARE_KEY_PASSWORD
//
// or a properties file named by AUDIO_SHARE_KEYSTORE_PROPERTIES /
// -PreleaseKeystoreProperties=... containing storeFile, storePassword,
// keyAlias, and keyPassword. The default candidate is outside the repo:
//   $HOME/.cache/audio-share-beta-keys/keystore.properties
// Production builds without credentials stay unsigned and fail at packaging;
// they never fall back to the debug key. Explicit test builds use debug signing.
val releaseKeystorePropertiesFile: File? =
    providers.gradleProperty("releaseKeystoreProperties").orNull?.let { file(it) }
        ?: providers.environmentVariable("AUDIO_SHARE_KEYSTORE_PROPERTIES").orNull?.let { file(it) }
        ?: providers.environmentVariable("HOME").orNull
            ?.let { file("$it/.cache/audio-share-beta-keys/keystore.properties") }
val releaseKeystoreProperties = Properties().apply {
    if (releaseKeystorePropertiesFile?.isFile == true) {
        releaseKeystorePropertiesFile.inputStream().use { load(it) }
    }
}
fun signingValue(envName: String, key: String): String? =
    providers.environmentVariable(envName).orNull
        ?: releaseKeystoreProperties.getProperty(key)?.takeIf { it.isNotBlank() }
val releaseStoreFile = signingValue("AUDIO_SHARE_KEYSTORE_FILE", "storeFile")?.let { file(it) }
val releaseStorePassword = signingValue("AUDIO_SHARE_KEYSTORE_PASSWORD", "storePassword")
val releaseKeyAlias = signingValue("AUDIO_SHARE_KEY_ALIAS", "keyAlias")
val releaseKeyPassword = signingValue("AUDIO_SHARE_KEY_PASSWORD", "keyPassword")
val hasReleaseSigning =
    releaseStoreFile?.isFile == true &&
        !releaseStorePassword.isNullOrBlank() &&
        !releaseKeyAlias.isNullOrBlank() &&
        !releaseKeyPassword.isNullOrBlank()

// TEST-ONLY. The fixture client identity key is compiled into the native
// library only when `-PtestSupport=true` is passed explicitly. Default
// release builds carry no test identity/signing path (production identities
// come from v2 pairing). E2E builds: ./gradlew assembleRelease
// -PtestSupport=true
val testSupport = providers.gradleProperty("testSupport")
    .map(String::toBoolean)
    .getOrElse(false)
// Device-test APKs carry test-only instrumentation and must never be signed
// with the customer release key. Both flags switch the release variant to the
// debug signing config (the same config as the app under test); production
// builds (no flags) use the dedicated release keystore.
val deviceTests = providers.gradleProperty("deviceTests")
    .map(String::toBoolean)
    .getOrElse(false)
val testOnlyBuild = testSupport || deviceTests

android {
    namespace = "dev.audioshare.v2"
    compileSdk = 35

    signingConfigs {
        if (hasReleaseSigning) {
            create("release") {
                storeFile = releaseStoreFile
                storePassword = releaseStorePassword
                keyAlias = releaseKeyAlias
                keyPassword = releaseKeyPassword
                // minSdk 26: APK Signature Scheme v2/v3 are sufficient.
                enableV1Signing = false
                enableV2Signing = true
                enableV3Signing = true
            }
        }
    }

    defaultConfig {
        applicationId = "dev.audioshare.v2"
        // ticket 08 gate: Android 8 / API 26 baseline, same as the prototype
        minSdk = 26
        targetSdk = 35
        // Beta release: applicationId dev.audioshare.v2 so the beta coexists
        // with the v1 app (ticket 21 acceptance).
        versionCode = 2
        versionName = "0.2.0-beta"
        testInstrumentationRunner = "dev.audioshare.v2.ControlSessionInstrumentation"
        ndk {
            // arm64-v8a is the real-device scope; x86_64 is the emulator ABI.
            // The beta universal APK carries both when -PandroidAbis is set.
            abiFilters += androidAbis
        }
        // TEST_SUPPORT gates every test-only identity/addressing code path
        // (adb extras and the native fixture identity). Production releases
        // are built without -PtestSupport and have this false.
        buildConfigField("boolean", "TEST_SUPPORT", testSupport.toString())
    }

    buildFeatures {
        buildConfig = true
        compose = true
    }

    testOptions {
        unitTests {
            // Robolectric screen tests need the merged manifest and resources.
            isIncludeAndroidResources = true
        }
    }

    buildTypes {
        release {
            // R8 on: proves the ProGuard rules the integration would need.
            // Production release builds require the dedicated keystore and
            // never fall back to the debug key. Explicit test builds
            // (`-PtestSupport` / `-PdeviceTests`) are signed with the debug
            // key so test artifacts can never carry the customer certificate.
            isMinifyEnabled = true
            isShrinkResources = true
            signingConfig = if (testOnlyBuild) {
                signingConfigs.getByName("debug")
            } else {
                signingConfigs.findByName("release")
            }
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
            if (deviceTests) {
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

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)

    // QR scanning without Google Play Services: ZXing decodes frames from a
    // CameraX ImageAnalysis pipeline wrapped in an AndroidView.
    implementation(libs.androidx.camera.core)
    implementation(libs.androidx.camera.camera2)
    implementation(libs.androidx.camera.lifecycle)
    implementation(libs.androidx.camera.view)
    implementation(libs.zxing.core)

    // Unit tests: the fixture replay harness. org.json comes from the real
    // artifact (android.jar's stubs throw in unit tests); kotlin("test")
    // provides the kotlin.test assertions.
    testImplementation(libs.junit)
    testImplementation("io.mockk:mockk:1.13.16")
    testImplementation("org.json:json:20240303")
    testImplementation(kotlin("test"))
    // Compose screen tests run on the JVM under Robolectric so the custom
    // release-build Instrumentation runner stays untouched.
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.core)
    testImplementation(libs.androidx.test.ext.junit)
    testImplementation(platform(libs.androidx.compose.bom))
    testImplementation(libs.androidx.compose.ui.test.junit4)
    testImplementation(libs.androidx.compose.ui.test.manifest)
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
// jniLibs. One build+strip pair is registered per selected ABI. The jniLibs
// source root stays build/generated/jniLibs so AGP sees the standard
// <abi>/lib*.so layout. Skipped with -PnoNative=true.

val nativeDir = rootProject.projectDir.resolve("native")
val jniLibsRoot = projectDir.resolve("build/generated/jniLibs")
val nativeLibName = "libaudio_share_client_v2_native.so"

val nativeStripTasks = androidAbis.map { abi ->
    val cargoTarget = abiToCargoTarget.getValue(abi)
    val taskSuffix = abi.replace("-", "").replace("_", "")
    val cargoOut = nativeDir.resolve("target/$cargoTarget/release")

    val buildNative = tasks.register<Exec>("buildNative$taskSuffix") {
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
        outputs.file(cargoOut.resolve(nativeLibName))
        inputs.dir(nativeDir.resolve("src"))
        inputs.file(nativeDir.resolve("Cargo.toml"))
    }

    tasks.register<Exec>("stripNative$taskSuffix") {
        onlyIf { !project.hasProperty("noNative") }
        dependsOn(buildNative)
        val so = cargoOut.resolve(nativeLibName)
        val dest = jniLibsRoot.resolve("$abi/$nativeLibName")
        inputs.file(so)
        outputs.file(dest)
        // Resolve ndkDir at execution time so Gradle sync/configuration does
        // not fail when the SDK environment is absent.
        doFirst {
            commandLine(
                "bash", "-c",
                "mkdir -p ${dest.parentFile.absolutePath} && " +
                    "cp ${so.absolutePath} ${dest.absolutePath} && " +
                    "${ndkDir.get()}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-strip " +
                    "${dest.absolutePath}"
            )
        }
    }
}

tasks.matching { it.name.startsWith("merge") && it.name.endsWith("JniLibFolders") }.configureEach {
    dependsOn(nativeStripTasks)
}

// Release APKs must be signed by the dedicated beta keystore. Fail packaging
// with an actionable message instead of silently producing an unsigned or
// debug-signed artifact.
tasks.matching { it.name == "packageRelease" }.configureEach {
    doFirst {
        if (!testOnlyBuild && !hasReleaseSigning) {
            throw GradleException(
                "Release signing is not configured. Set AUDIO_SHARE_KEYSTORE_FILE " +
                    "(or AUDIO_SHARE_KEYSTORE_PROPERTIES) and the matching password/alias " +
                    "variables. Release builds no longer fall back to the debug key."
            )
        }
    }
}
