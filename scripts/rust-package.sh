#!/usr/bin/env bash
set -euo pipefail

# 打包 Rust 发行版：scripts/rust-package.sh v<VERSION> [输出目录]
# 产物：muke-<VERSION>-<arch>.tar.gz（二进制 + 前端 + 插件前端 + 配置示例与 systemd 单元）
# 需要先在目标架构上完成 release 构建（cargo build --release --locked）。

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
if [[ ! -x "$ROOT/target/release/muke" ]]; then
    echo "缺少 target/release/muke；先运行 cargo build --release --locked" >&2
    exit 1
fi
if [[ "$("$ROOT/target/release/muke" --version)" != "muke $VERSION" ]]; then
    echo "二进制版本与 VERSION 不一致，请重新构建" >&2
    exit 1
fi
OUTPUT="${2:-$ROOT/build/release}"
mkdir -p "$OUTPUT"
OUTPUT="$(cd "$OUTPUT" && pwd)"
ARCH="$(uname -m)"
ARCHIVE="muke-$VERSION-$ARCH.tar.gz"
if [[ -e "$OUTPUT/$ARCHIVE" || -e "$OUTPUT/SHA256SUMS-$ARCH" ]]; then
    echo "发布产物已存在，请使用新的输出目录" >&2
    exit 1
fi

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
PACKAGE="$STAGE/muke-$VERSION"
mkdir -p "$PACKAGE"
install -m 0755 "$ROOT/target/release/muke" "$PACKAGE/muke"
cp -a "$ROOT/frontend" "$PACKAGE/frontend"
# 运行时只需要插件前端资源与清单目录结构
mkdir -p "$PACKAGE/plugins"
for plugin in "$ROOT"/plugins/*/; do
    id="$(basename "$plugin")"
    if [[ -d "$plugin/frontend" ]]; then
        mkdir -p "$PACKAGE/plugins/$id"
        cp -a "$plugin/frontend" "$PACKAGE/plugins/$id/frontend"
    fi
done
install -m 0644 "$ROOT/scripts/muke.toml.example" "$PACKAGE/muke.toml.example"
install -m 0644 "$ROOT/scripts/muke.service" "$PACKAGE/muke.service"
install -m 0644 "$ROOT/LICENSE" "$PACKAGE/LICENSE"
install -m 0644 "$ROOT/VERSION" "$PACKAGE/VERSION"
install -m 0644 "$ROOT/CHANGELOG.md" "$PACKAGE/CHANGELOG.md"
mkdir -p "$PACKAGE/docs"
install -m 0644 "$ROOT/docs/rust-install.md" "$PACKAGE/docs/rust-install.md"

tar -C "$STAGE" -czf "$OUTPUT/$ARCHIVE" "muke-$VERSION"
(cd "$OUTPUT" && sha256sum "$ARCHIVE" > "SHA256SUMS-$ARCH")
echo "Rust 安装包：$OUTPUT/$ARCHIVE"
