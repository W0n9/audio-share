#!/usr/bin/env bash
# 只上传已核对哈希的资产；永远不创建标签或直接发布。
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$ROOT/scripts/lib.sh"
[[ $# -eq 4 ]] || fail '用法：create-draft-release.sh <artifacts-dir> <version> <apk-name> <commit-sha>'
release_dir=$1
version=$2
apk_name=$3
commit=$4
bash "$ROOT/scripts/check-release-version.sh" "$version" >/dev/null
[[ "$apk_name" =~ ^audio-share-client-v2-[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+([.-][0-9A-Za-z]+)*)?\.apk$ ]] ||
    fail "无效的 APK 资产名：$apk_name"
notes="$ROOT/docs/releases/v${version}.md"
[[ -s "$notes" ]] || fail "缺少 GitHub 发布说明：$notes"

assets=()
for path in \
    "audio-share-client-v2-android/$apk_name" \
    'AudioShareServer-windows/AudioShareServer.exe' \
    'audio-share-server-cmd-windows/audio-share-server-cmd-windows.zip' \
    "audio-share-server-deb/AudioShareServer_${version}_amd64.deb" \
    "audio-share-server-appimage/AudioShareServer_${version}_amd64.AppImage" \
    'audio-share-server-cmd-linux/audio-share-server-cmd-linux.tar.gz'; do
    file="$release_dir/$path"
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
