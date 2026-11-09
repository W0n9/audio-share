#!/usr/bin/env bash
# Set the SourceForge per-platform default downloads for a release.
#
# 0.4.0 is Server-only: the Android Client remains v0.3.4 and no new APK is
# attached to the release, so no android default is set here.
api_key=$1
version=$2

if [ -z "$api_key" ] || [ -z "$version" ]; then
    echo 'error arg'
    exit 1
fi

do_put() {
    api_key=$1
    default=$2
    project_name=$3
    file_path=$4
    echo setup "$default" "$project_name" "\"$file_path\""
    curl -H "Accept: application/json" -X PUT \
        -d "default=$default" \
        -d "api_key=$api_key" \
        "https://sourceforge.net/projects/$project_name/files/$file_path"
    printf '\n'
}

# Primary download per platform. The versioned deb/AppImage names are produced
# by scripts/package-linux-artifacts.sh; the Windows artifacts are versionless.
do_put "$api_key" windows audio-share "v$version/AudioShareServer.exe"
do_put "$api_key" linux audio-share "v$version/AudioShareServer_${version}_amd64.AppImage"
do_put "$api_key" others audio-share "v$version/v$version%20source%20code.tar.gz"
