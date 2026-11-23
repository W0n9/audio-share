#!/usr/bin/env bash
# 在隔离仓库中检验发版输入、APK 打包、标签绑定和 draft 参数；不访问 GitHub。
set -euo pipefail

src=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
source "$src/lib.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cd "$tmp"
mkdir -p scripts docs/releases fake-bin apk dist
cp "$src"/{lib,get_version,check-release-version,check-release-tag,package-android-release,create-draft-release}.sh scripts/

expect_fail() {
    local message=$1
    shift
    if "$@" >out.log 2>error.log; then
        fail "命令应失败：$*"
    fi
    [[ -z "$message" ]] || grep -Fq "$message" error.log || fail "没有报告预期问题：$message"
}

# 版本与 draft 说明不依赖 v1 / F-Droid 元数据。
for version in 0.6.0 0.6.0-rc.1 0.6.0-beta; do
    printf '%s\n' "$version" >VERSION
    [[ $(bash scripts/check-release-version.sh "$version") == "version=$version" ]]
done
expect_fail '不一致' bash scripts/check-release-version.sh 0.6.0
printf '0.06.0\n' >VERSION
expect_fail '不是受支持的发布版本' bash scripts/check-release-version.sh 0.06.0
printf '0.6.0\n' >VERSION

# aapt 与 apksigner 使用替身，APK 是真实 ZIP；测试 DEX 的流式读取和打包输出。
cat >fake-bin/apksigner <<'EOF'
#!/usr/bin/env bash
printf 'Signer #1 certificate DN: CN=%s\n' "${FAKE_CERT:-Audio Share Release}"
EOF
cat >fake-bin/aapt <<'EOF'
#!/usr/bin/env bash
printf "package: name='dev.audioshare.v2' versionCode='%s' versionName='0.6.0' compileSdkVersion='37'\n" "${FAKE_CODE:-3}"
printf 'native-code: %s\n' "'${FAKE_ABI:-arm64-v8a}'"
EOF
cat >apk/output-metadata.json <<'EOF'
{"applicationId":"dev.audioshare.v2","variantName":"release","artifactType":{"type":"APK"},"elements":[{"type":"SINGLE","filters":[],"outputFile":"app-release.apk","versionCode":3,"versionName":"0.6.0"}]}
EOF
chmod +x fake-bin/{apksigner,aapt}
printf 'production dex bytes\n' >classes.dex
zip -q apk/app-release.apk classes.dex
apk_name=$(bash scripts/package-android-release.sh fake-bin apk dist arm64-v8a)
[[ "$apk_name" == audio-share-client-v2-0.6.0-arm64-v8a.apk ]]
(cd dist && sha256sum -c "$apk_name.sha256")
expect_fail 'Gradle 元数据不一致' env FAKE_CODE=2 bash scripts/package-android-release.sh fake-bin apk dist arm64-v8a
expect_fail 'APK 的 ABI 不是 x86_64' bash scripts/package-android-release.sh fake-bin apk dist x86_64
expect_fail '不支持的发布 ABI' bash scripts/package-android-release.sh fake-bin apk dist armeabi-v7a
expect_fail 'Debug 证书' env FAKE_CERT='Android Debug' bash scripts/package-android-release.sh fake-bin apk dist arm64-v8a
printf 'TestSupportHarness\n' >classes2.dex
for ((i = 0; i < 2048; ++i)); do
    printf '%0128d\n' "$i"
done >>classes2.dex
zip -q apk/app-release.apk classes2.dex
expect_fail '包含 test-support' bash scripts/package-android-release.sh fake-bin apk dist arm64-v8a
# 保留 APK 容器但无 DEX，unzip 的非零退出必须拒绝产物。
printf 'not dex\n' >manifest.bin
rm -f apk/app-release.apk
zip -q apk/app-release.apk manifest.bin
expect_fail '' bash scripts/package-android-release.sh fake-bin apk dist arm64-v8a

# origin 指向本地仓库，覆盖 lightweight、annotated、缺失与指向旧 SHA 四种情况。
git init -q
git config user.email ci-test@example.invalid
git config user.name 'Release test'
git config commit.gpgsign false
git config tag.gpgsign false
git remote add origin .
git commit -q --allow-empty -m first
old=$(git rev-parse HEAD)
git tag v0.6.0-old "$old"
git commit -q --allow-empty -m build
commit=$(git rev-parse HEAD)
git tag -am stable v0.6.0 "$commit"
git tag v0.6.0-rc.1 "$commit"
bash scripts/check-release-tag.sh v0.6.0 "$commit"
bash scripts/check-release-tag.sh v0.6.0-rc.1 "$commit"
expect_fail '拒绝创建 draft' bash scripts/check-release-tag.sh v0.6.0-old "$commit"
expect_fail 'origin 没有' bash scripts/check-release-tag.sh v0.6.2 "$commit"

# 捕获 gh 参数，确保只上传验证过的资产，且不创建新标签或发布。
cat >fake-bin/gh <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$@" >gh.args
EOF
chmod +x fake-bin/gh
export PATH="$tmp/fake-bin:$PATH" GITHUB_REPOSITORY=example/release-test
expect_fail '缺少 GitHub 发布说明' bash scripts/create-draft-release.sh assets 0.6.0 "$commit"

fixture_asset() {
    mkdir -p "$(dirname "$1")"
    printf 'fixture %s\n' "$1" >"$1"
    write_sha256 "$1"
}

for version in 0.6.0 0.6.0-rc.1; do
    printf '%s\n' "$version" >VERSION
    printf 'Notes for %s\n' "$version" >"docs/releases/v$version.md"
    rm -rf assets/audio-share-client-v2-android
    for abi in arm64-v8a x86_64; do
        fixture_asset "assets/audio-share-client-v2-android/audio-share-client-v2-${version}-${abi}.apk"
    done
    for path in \
        'AudioShareServer-windows/AudioShareServer.exe' \
        'audio-share-server-cmd-windows/audio-share-server-cmd-windows.zip' \
        "audio-share-server-deb/AudioShareServer_${version}_amd64.deb" \
        "audio-share-server-appimage/AudioShareServer_${version}_amd64.AppImage" \
        'audio-share-server-cmd-linux/audio-share-server-cmd-linux.tar.gz'; do
        fixture_asset "assets/$path"
    done
    fixture_asset assets/unexpected/unexpected.zip
    bash scripts/create-draft-release.sh assets "$version" "$commit"
    grep -Fxq -- '--draft' gh.args
    grep -Fxq -- '--verify-tag' gh.args
    ! grep -Fxq -- '--target' gh.args || fail 'draft 不得设置用于创建标签的 --target'
    [[ $(grep -c '^assets/' gh.args) == 14 ]] || fail '应上传 7 个文件及其校验和'
    ! grep -q 'unexpected.zip' gh.args || fail '上传了未验证资产'
    if [[ "$version" == *-* ]]; then
        grep -Fxq -- '--prerelease' gh.args
    else
        ! grep -Fxq -- '--prerelease' gh.args || fail '稳定版不能标记为预发布'
    fi

    # 每个架构都必须有 APK；缺一个就失败。
    rm "assets/audio-share-client-v2-android/audio-share-client-v2-${version}-x86_64.apk" \
        "assets/audio-share-client-v2-android/audio-share-client-v2-${version}-x86_64.apk.sha256"
    rm -f gh.args
    expect_fail 'missing build input' bash scripts/create-draft-release.sh assets "$version" "$commit"
    [[ ! -e gh.args ]] || fail '缺 APK 时不能调用 gh'
    fixture_asset "assets/audio-share-client-v2-android/audio-share-client-v2-${version}-x86_64.apk"
    fixture_asset "assets/audio-share-client-v2-android/audio-share-client-v2-${version}-universal.apk"
    expect_fail '未预期的 APK' bash scripts/create-draft-release.sh assets "$version" "$commit"
    rm "assets/audio-share-client-v2-android/audio-share-client-v2-${version}-universal.apk"*
done
rm -f gh.args
printf '0.6.0\n' >VERSION
rm -rf assets/audio-share-client-v2-android
for abi in arm64-v8a x86_64; do
    fixture_asset "assets/audio-share-client-v2-android/audio-share-client-v2-0.6.0-${abi}.apk"
done
printf 'invalid hash\n' >"assets/audio-share-client-v2-android/audio-share-client-v2-0.6.0-arm64-v8a.apk.sha256"
expect_fail 'SHA-256 或校验和文件名不匹配' bash scripts/create-draft-release.sh assets 0.6.0 "$commit"
[[ ! -e gh.args ]] || fail '校验失败时不能调用 gh'

printf 'release-workflow self-test: PASS\n'