#!/usr/bin/env bash
# 校验签名 APK 与 Gradle 的实际产物元数据，输出发布资产名。
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
[[ $# -eq 3 ]] || fail '用法：package-android-release.sh <build-tools-dir> <apk-dir> <out-dir>'
tools=$1
apk_dir=$2
out_dir=$3
apk="$apk_dir/app-release.apk"
metadata="$apk_dir/output-metadata.json"
require_file "$apk"
require_file "$metadata"

# 用 Gradle 的 release 产物元数据，不解析 Kotlin 源码，也不把 Android 版本绑到 Server。
version_name=$(jq -er '
    select(.applicationId == "dev.audioshare.v2" and .variantName == "release" and
           .artifactType.type == "APK" and (.elements | length) == 1) |
    .elements[0] | select(.type == "SINGLE" and .filters == [] and .outputFile == "app-release.apk") |
    .versionName | select(type == "string" and test("^[0-9]+\\.[0-9]+\\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?\\z"))
' "$metadata") || fail 'Gradle 元数据不是预期的单一 release APK 或 versionName 不可用于发布资产名'
version_code=$(jq -er '.elements[0].versionCode | select(type == "number" and . > 0 and floor == .)' "$metadata") ||
    fail 'Gradle 元数据中的 Android versionCode 无效'

certs=$("$tools/apksigner" verify --verbose --print-certs "$apk") || fail 'APK 签名校验失败'
if grep -qi '^Signer .* certificate DN:.*CN=Android Debug' <<<"$certs"; then
    fail '发布 APK 不得使用 Android Debug 证书'
fi
badging=$("$tools/aapt" dump badging "$apk") || fail '无法读取 APK 元数据'
expected="package: name='dev.audioshare.v2' versionCode='$version_code' versionName='$version_name'"
package=$(sed -n '/^package: /p' <<<"$badging")
[[ "$package" == "$expected" || "$package" == "$expected "* ]] ||
    fail 'APK 包名或版本与 Gradle 元数据不一致'

# aapt 可能以不同顺序列出 ABI，但必须恰好包含这两种。
abis=$(sed -n 's/^native-code: //p' <<<"$badging" | tr ' ' '\n' | LC_ALL=C sort)
[[ "$abis" == $'\'arm64-v8a\'\n\'x86_64\'' ]] || fail "APK 并非预期的双 ABI：$abis"

# 必须读完整个 DEX：grep -q 在 pipefail 下会提前关管道，可能漏掉匹配。
# 先确认解包/strings 成功，不能把管道失败误当作不存在测试路径。
dex_strings=$(mktemp)
trap 'rm -f "$dex_strings"' EXIT
unzip -p "$apk" 'classes*.dex' | strings >"$dex_strings"
if grep -qE 'TestSupportHarness|pinnedSpki|serverKey|smoke-client' "$dex_strings"; then
    fail '生产 APK 中包含 test-support 路径'
fi

name="audio-share-client-v2-${version_name}.apk"
mkdir -p "$out_dir"
cp "$apk" "$out_dir/$name"
write_sha256 "$out_dir/$name"
printf '%s\n' "$name"
