#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VERSION="$(git show HEAD:VERSION)"
if [[ ! "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
  echo "VERSION 必须为 major.minor.patch" >&2
  exit 1
fi
if [[ "${1:-}" != "v$VERSION" ]]; then
  echo "标签与 VERSION 不一致：期望 v$VERSION" >&2
  exit 1
fi
OUTPUT="${2:-$ROOT/build/release}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
ARCHIVE="muke-$VERSION.zip"
if [[ -e "$OUTPUT/$ARCHIVE" || -e "$OUTPUT/SHA256SUMS" ]]; then
  echo "发布产物已存在，请使用新的输出目录" >&2
  exit 1
fi

# 从提交白名单打包，不运行应用，不读取工作区中的个人配置或数据。
git archive --format=zip --prefix="muke-$VERSION/" --output="$OUTPUT/$ARCHIVE" HEAD -- \
  VERSION CHANGELOG.md LICENSE README.md ARCHITECTURE.md docs \
  app.php config.default.php src public lib plugins bin \
  Dockerfile .dockerignore docker-compose.yml ecosystem.config.cjs netlify.toml tests/restore-rehearsal.php
(cd "$OUTPUT" && sha256sum "$ARCHIVE" > SHA256SUMS)
echo "项目安装包：$OUTPUT/$ARCHIVE"
