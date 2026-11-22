#!/usr/bin/env bash
# 校验手动输入的产品版本，避免在旧提交上误打包另一版。
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$ROOT/scripts/lib.sh"
[[ $# -eq 1 ]] || fail '用法：check-release-version.sh <预期版本>'
version=$(bash "$ROOT/scripts/get_version.sh" -n)
# 与项目的版本号规则一致，并禁止路径符号与前导零。
pattern='^(0|[1-9][0-9]{0,2})\.(0|[1-9][0-9]?)\.(0|[1-9][0-9]?)(-(alpha|beta|rc)(\.(0|[1-9][0-9]?))?)?$'
[[ "$version" =~ $pattern ]] || fail "VERSION 不是受支持的发布版本：$version"
bash "$ROOT/scripts/get_version.sh" -c >/dev/null
[[ "$1" == "$version" ]] || fail "输入版本 $1 与所选提交的 VERSION=$version 不一致"
printf 'version=%s\n' "$version"
