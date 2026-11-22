#!/usr/bin/env bash
# 确认 origin 上的发布标签（包括 annotated tag）指向实际构建提交。
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"
[[ $# -eq 2 ]] || fail '用法：check-release-tag.sh <tag> <commit-sha>'
tag=$1
commit=$2
ref="refs/tags/$tag"
git check-ref-format "$ref" || fail "无效的发布标签：$tag"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || fail '构建提交必须是完整的 commit SHA'

# annotated tag 有单独的标签对象；^{} 才是它指向的对象。
refs=$(git ls-remote --tags origin "$ref" "$ref^{}")
target=$(awk -v ref="$ref" '$2 == ref { direct = $1 } $2 == ref "^{}" { peeled = $1 }
    END { print (peeled != "" ? peeled : direct) }' <<<"$refs")
[[ -n "$target" ]] || fail "origin 没有 $tag；创建 draft 前须先推送指向正式构建提交的标签"
[[ "$target" == "$commit" ]] || fail "$tag 指向 $target，但构建来自 $commit；拒绝创建 draft"
