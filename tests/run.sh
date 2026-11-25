#!/usr/bin/env bash
# 冒烟测试总入口：bash tests/run.sh
set -e
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PHP=/usr/bin/php
PORT=8771

echo "== Lint =="
for f in "$ROOT"/app.php "$ROOT"/public/index.php "$ROOT"/config.default.php "$ROOT"/bin/*.php "$ROOT"/plugins/*/plugin.php; do
  [ -f "$f" ] && "$PHP" -l "$f" >/dev/null
done
for f in "$ROOT"/src/*.php "$ROOT"/src/*/*.php; do
  [ -f "$f" ] && "$PHP" -l "$f" >/dev/null
done
node --input-type=module --check < "$ROOT/public/assets/app.js" >/dev/null
node --check "$ROOT/public/assets/registry.js" >/dev/null
node --check "$ROOT/public/assets/embed-init.js" >/dev/null
for f in "$ROOT"/public/assets/lib/*.js "$ROOT"/public/assets/views/*.js "$ROOT"/public/assets/components/*.js "$ROOT"/plugins/*/plugin.js; do
  [ -f "$f" ] && node --input-type=module --check < "$f" >/dev/null
done
echo "lint OK"

echo "== 项目安装包 =="
bash "$ROOT/tests/package.sh"

echo "== 沙箱 =="
SBOX=$(mktemp -d)
mkdir -p "$SBOX/walls"
cp -r "$ROOT"/VERSION "$ROOT"/app.php "$ROOT"/bin "$ROOT"/public "$ROOT"/lib "$ROOT"/plugins "$ROOT"/config.default.php "$SBOX"/
[ -d "$ROOT/src" ] && cp -r "$ROOT"/src "$SBOX"/
command -v magick >/dev/null && magick -size 400x300 gradient:gray "$SBOX/walls/w1.jpg"
echo "<?php return ['wallpaper_dir' => '$SBOX/walls'];" > "$SBOX/config.php"
kill "$(ss -tlnp 2>/dev/null | grep ":$PORT " | grep -o 'pid=[0-9]*' | head -1 | cut -d= -f2)" 2>/dev/null || true
(cd "$SBOX" && MKBLOG_DATA_DIR="$SBOX/var" "$PHP" -S 127.0.0.1:$PORT -t public public/index.php >/dev/null 2>&1 &
 echo $! > "$SBOX/server.pid")
for i in $(seq 1 30); do
  curl -s -m 1 "http://127.0.0.1:$PORT/api?action=state" | grep -q csrf && break
  sleep 0.3
done
cleanup() { kill "$(cat "$SBOX/server.pid")" 2>/dev/null || true; rm -rf "$SBOX"; }
trap cleanup EXIT

echo "== API =="
"$PHP" "$ROOT/tests/article-summary.php"
node "$ROOT/tests/http.cjs"
bash "$ROOT/tests/api.sh" "http://127.0.0.1:$PORT"
node "$ROOT/tests/wallpaper-upload.cjs" "http://127.0.0.1:$PORT" "$SBOX"
echo "== 浏览器 =="
node "$ROOT/tests/smoke.cjs" "http://127.0.0.1:$PORT"
node "$ROOT/tests/reliability.cjs" "http://127.0.0.1:$PORT" "$SBOX"
node "$ROOT/tests/tasks-pinning.cjs" "http://127.0.0.1:$PORT" "$SBOX"
node "$ROOT/tests/article-pagination.cjs" "http://127.0.0.1:$PORT" "$SBOX"
node "$ROOT/tests/article-revisions.cjs" "http://127.0.0.1:$PORT" "$SBOX"
node "$ROOT/tests/editor-protection.cjs" "http://127.0.0.1:$PORT" "$SBOX"
node "$ROOT/tests/feed-search.cjs" "http://127.0.0.1:$PORT" "$SBOX"
node "$ROOT/tests/rss-tab.cjs" "http://127.0.0.1:$PORT" "$SBOX"
node "$ROOT/tests/giscus.cjs" "http://127.0.0.1:$PORT" "$SBOX"
echo "== 导出 =="
(cd "$SBOX" && "$PHP" bin/export.php >/dev/null)
sqlite3 "$SBOX/var/blog.sqlite" "INSERT INTO settings (key,value) VALUES ('comment_provider','custom'),('comment_embed_url','https://example.com/comments') ON CONFLICT(key) DO UPDATE SET value=excluded.value;" 2>/dev/null || true
(cd "$SBOX" && "$PHP" bin/export.php >/dev/null)
grep -q '冒烟评论' "$SBOX/dist/article/1/index.html" || { echo "静态烘焙评论缺失"; exit 1; }
grep -q 'example.com/comments' "$SBOX/dist/article/1/index.html" || { echo "第三方评论 iframe 缺失"; exit 1; }
grep -q 'frame-src' "$SBOX/dist/_headers" || { echo "_headers frame-src 缺失"; exit 1; }
sqlite3 "$SBOX/var/blog.sqlite" "UPDATE settings SET value='waline' WHERE key='comment_provider'; UPDATE settings SET value='https://wl.example.com' WHERE key='comment_embed_url';"
(cd "$SBOX" && "$PHP" bin/export.php >/dev/null)
test -f "$SBOX/dist/embed/comments/index.html" || { echo "Waline 嵌入页缺失"; exit 1; }
grep -q 'unpkg.com' "$SBOX/dist/_headers" || { echo "嵌入页 CSP 分区缺失"; exit 1; }
[ -f "$SBOX/dist/rss.xml" ] && [ -f "$SBOX/dist/sitemap.xml" ] && [ -f "$SBOX/dist/index.html" ] && echo "export OK"

echo "== 插件契约 =="
mkdir -p "$SBOX/plugins/contract"
cp "$ROOT/tests/fixtures/plugin.php" "$SBOX/plugins/contract/plugin.php"
"$PHP" "$ROOT/tests/plugin-contract.php" "$SBOX"
"$PHP" "$ROOT/tests/rss-plugin.php" "$SBOX"
"$PHP" "$ROOT/tests/dms-plugin.php" "$SBOX"
"$PHP" "$ROOT/tests/deploy-sync.php" "$SBOX"
"$PHP" "$ROOT/tests/deploy-spawn.php" "$SBOX"
"$PHP" "$ROOT/tests/sync-scheduler.php" "$SBOX" auto-deploy-test
"$PHP" "$ROOT/tests/scheduled-backup.php" "$SBOX"
"$PHP" "$ROOT/tests/backup-export.php" "$SBOX"

echo "== 无插件导出 =="
mv "$SBOX/plugins" "$SBOX/plugins-disabled"
mkdir "$SBOX/plugins"
(cd "$SBOX" && MKBLOG_DATA_DIR="$SBOX/no-plugins-data" "$PHP" bin/export.php >/dev/null)
test ! -f "$SBOX/dist/wallpapers/index.html"

echo "ALL SMOKE PASSED"
