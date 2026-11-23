#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMP="$(mktemp -d)"
trap 'rm -rf "$TEMP"' EXIT
REPO="$TEMP/repo"
mkdir -p "$REPO/scripts" "$REPO/tests" "$REPO/var" "$REPO/dist" "$REPO/.github"
cp -r "$ROOT"/VERSION "$ROOT"/CHANGELOG.md "$ROOT"/LICENSE "$ROOT"/README.md "$ROOT"/ARCHITECTURE.md \
  "$ROOT"/docs "$ROOT"/app.php "$ROOT"/config.default.php "$ROOT"/src "$ROOT"/public \
  "$ROOT"/lib "$ROOT"/plugins "$ROOT"/bin "$ROOT"/Dockerfile "$ROOT"/.dockerignore \
  "$ROOT"/docker-compose.yml "$ROOT"/ecosystem.config.cjs "$ROOT"/netlify.toml "$REPO/"
cp "$ROOT/scripts/package.sh" "$REPO/scripts/"
cp "$ROOT/tests/restore-rehearsal.php" "$REPO/tests/"
# 故意把假私有文件提交到测试仓库，验证白名单而不只依赖 gitignore。
for file in config.php var/blog.sqlite var/private.jpg dist/index.html .github/private.txt .env; do
  printf 'package-test-private\n' > "$REPO/$file"
done
git -C "$REPO" init -q
git -C "$REPO" add .
git -C "$REPO" -c user.name='Package Test' -c user.email='package-test@example.invalid' -c commit.gpgsign=false commit -qm 'Package fixture'
VERSION="$(< "$REPO/VERSION")"
bash "$REPO/scripts/package.sh" "v$VERSION" "$TEMP/output"
(cd "$TEMP/output" && sha256sum -c SHA256SUMS)
unzip -q "$TEMP/output/muke-$VERSION.zip" -d "$TEMP/unpacked"
APP="$TEMP/unpacked/muke-$VERSION"
for file in VERSION app.php src/Database.php public/index.php lib/Parsedown.php plugins/wallpapers/plugin.php bin/backup.php tests/restore-rehearsal.php; do
  test -f "$APP/$file"
done
for file in config.php var dist .git .github .env; do
  test ! -e "$APP/$file"
done
MKBLOG_DATA_DIR="$TEMP/data" php -r 'require $argv[1] . "/app.php"; if (MKBLOG_VERSION !== $argv[2]) exit(1); echo "安装包启动与版本检查通过\n";' "$APP" "$VERSION"
if bash "$REPO/scripts/package.sh" v999.0.0 "$TEMP/rejected" 2>/dev/null; then
  echo '错误标签未被拒绝' >&2
  exit 1
fi
if bash "$REPO/scripts/package.sh" "v$VERSION" "$TEMP/output" 2>/dev/null; then
  echo '已存在的安装包被覆盖' >&2
  exit 1
fi
echo 'package: 安装包内容、敏感文件排除、校验和、解压启动及标签验证通过'
