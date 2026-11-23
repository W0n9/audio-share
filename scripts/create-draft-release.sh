#!/usr/bin/env bash
# 只上传已核对哈希的资产；永远不创建标签或直接发布。
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$ROOT/scripts/lib.sh"
[[ $# -eq 3 ]] || fail '用法：create-draft-release.sh <artifacts-dir> <version> <commit-sha>'
release_dir=$1
version=$2
commit=$3
bash "$ROOT/scripts/check-release-version.sh" "$version" >/dev/null
notes="$ROOT/docs/releases/v${version}.md"
[[ -s "$notes" ]] || fail "缺少 GitHub 发布说明：$notes"

# Android 每架构一个 APK；两个都必须存在，且目录里不能混入其它 APK。
apk_dir="$release_dir/audio-share-client-v2-android"
apk_names=()
for abi in arm64-v8a x86_64; do
    name="audio-share-client-v2-${version}-${abi}.apk"
    require_file "$apk_dir/$name"
    apk_names+=("$name")
done
extra_apks=$(find "$apk_dir" -maxdepth 1 -name '*.apk' ! -name "audio-share-client-v2-${version}-arm64-v8a.apk" \
    ! -name "audio-share-client-v2-${version}-x86_64.apk")
[[ -z "$extra_apks" ]] || fail "存在未预期的 APK：$extra_apks"

assets=()
for path in \
    "${apk_names[0]}" "${apk_names[1]}" \
    'AudioShareServer-windows/AudioShareServer.exe' \
    'audio-share-server-cmd-windows/audio-share-server-cmd-windows.zip' \
    "audio-share-server-deb/AudioShareServer_${version}_amd64.deb" \
    "audio-share-server-appimage/AudioShareServer_${version}_amd64.AppImage" \
    'audio-share-server-cmd-linux/audio-share-server-cmd-linux.tar.gz'; do
    case "$path" in
    audio-share-client-v2-*.apk) file="$apk_dir/$path" ;;
    *) file="$release_dir/$path" ;;
    esac
    require_file "$file"
    require_file "$file.sha256"
    sum=$(sha256sum -- "$file")
    [[ "$(<"$file.sha256")" == "${sum%% *}  ${path##*/}" ]] || fail "SHA-256 或校验和文件名不匹配：$path"
    assets+=("$file" "$file.sha256")
done

# 构建可能耗时；创建 draft 前再检查一遍远端，而不是信任本地缓存。
tag="v$version"
bash "$ROOT/scripts/check-release-tag.sh" "$tag" "$commit"
flags=()
if [[ "$version" == *-* ]]; then
    flags+=(--prerelease)
fi
gh release create "$tag" --repo "$GITHUB_REPOSITORY" --draft --verify-tag \
    "${flags[@]}" --notes-file "$notes" --title "$tag" "${assets[@]}"