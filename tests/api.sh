#!/usr/bin/env bash
# API 冒烟：bash tests/api.sh <base_url>
set -e
BASE="$1"
J=$(mktemp)
trap 'rm -f "$J"' EXIT

CSRF=$(curl -s -c "$J" "$BASE/api?action=state" | python3 -c 'import sys,json;print(json.load(sys.stdin)["csrf"])')
curl -s -b "$J" -c "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"username":"tester","password":"Password12345"}' "$BASE/api?action=setup" >/dev/null

# 动态：发表 + 非法内容拒绝
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"content":"冒烟动态"}' "$BASE/api?action=save-post" | grep -q '"ok":true'
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"content":"'"$(python3 -c 'print("长"*600)')"'"}' "$BASE/api?action=save-post" | grep -q '最多'; then :; else echo "字数限制未生效"; exit 1; fi

# 文章：保存 + 标签
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"title":"冒烟文章","content":"正文","format":"md","tags":"测试,冒烟"}' "$BASE/api?action=save-article" | grep -q '"ok":true'
curl -s -b "$J" "$BASE/api?action=articles" | grep -q '冒烟文章'

# 设置：读写 + 非法拒绝
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"site_tagline":"冒烟标语"}' "$BASE/api?action=save-settings" | grep -q '"ok":true'
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"post_max_chars":"99999"}' "$BASE/api?action=save-settings" | grep -q '之间'; then :; else echo "参数校验未生效"; exit 1; fi

# 插件：列表 + 同步（wallpapers 恒存在）
curl -s -b "$J" "$BASE/api?action=plugins" | grep -q '"wallpapers"'

# 插件清空动态：删除帖子与墓碑、保留配置
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"id":"wallpapers"}' "$BASE/api?action=purge-plugin-posts" | grep -q '"ok":true'
sqlite3_check=$(curl -s -b "$J" "$BASE/api?action=posts" | python3 -c 'import sys,json;print(sum(1 for p in json.load(sys.stdin)["posts"] if p.get("source")=="wallpapers"))')
[ "$sqlite3_check" = "0" ] || { echo "插件动态未清空"; exit 1; }

# 评论：公开提交 → 列表可见 → 蜜罐拦截 → 标垃圾后不可见 → 管理列表计数
CID=$(curl -s -b "$J" -c "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"kind":"article","target":1,"author":"访客甲","content":"冒烟评论，坐个沙发","website_url":""}' \
  "$BASE/api?action=comment" | python3 -c 'import sys,json;json.load(sys.stdin);print("ok")')
[ "$CID" = "ok" ] || { echo "评论提交失败"; exit 1; }
curl -s "$BASE/api?action=comments&kind=article&target=1" | grep -q '冒烟评论' || { echo "评论不可见"; exit 1; }
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"kind":"article","target":1,"author":"bot","content":"spam","website_url":"http://spam"}' \
  "$BASE/api?action=comment" | grep -q '"ok":true' || { echo "蜜罐提交应假成功"; exit 1; }
curl -s "$BASE/api?action=comments&kind=article&target=1" | grep -qv 'http://spam' || true
MID=$(curl -s -b "$J" "$BASE/api?action=comments-admin&status=all" | python3 -c 'import sys,json;print(json.load(sys.stdin)["comments"][0]["id"])')
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d "{\"id\":$MID,\"op\":\"spam\"}" "$BASE/api?action=moderate-comment" | grep -q '"ok":true'
curl -s "$BASE/api?action=comments&kind=article&target=1" | grep -q '冒烟评论' && { echo "标垃圾后仍可见"; exit 1; } || true
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d "{\"id\":$MID,\"op\":\"approve\"}" "$BASE/api?action=moderate-comment" | grep -q '"ok":true'

# 评论系统设置：合法回写 + 非法拒绝 + 还原原生
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_provider":"waline","comment_embed_url":"https://wl.example.com"}' "$BASE/api?action=save-settings" | grep -q '"ok":true'
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_provider":"disqus"}' "$BASE/api?action=save-settings" | grep -q '取值无效'; then :; else echo "评论系统取值校验未生效"; exit 1; fi
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_provider":"","comment_embed_url":""}' "$BASE/api?action=save-settings" >/dev/null

# 频控窗口置 0（便于连续提交测试）
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_rate_seconds":0}' "$BASE/api?action=save-settings" >/dev/null

# 关闭评论：提交被拒
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_provider":"off"}' "$BASE/api?action=save-settings" >/dev/null
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"kind":"article","target":1,"author":"关","content":"不应成功"}' "$BASE/api?action=comment" | grep -q '已关闭'; then :; else echo "关闭评论未生效"; exit 1; fi

# 仅文章范围：动态评论被拒
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_provider":"","comment_scope":"article"}' "$BASE/api?action=save-settings" >/dev/null
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"kind":"post","target":1,"author":"关","content":"不应成功"}' "$BASE/api?action=comment" | grep -q '已关闭评论'; then :; else echo "仅文章范围未生效"; exit 1; fi

# 先审后发：提交成功但不可见，进待审队列
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_moderation":"pre"}' "$BASE/api?action=save-settings" >/dev/null
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"kind":"article","target":1,"author":"预审","content":"待审内容"}' "$BASE/api?action=comment" | grep -q '"ok":true'
if curl -s "$BASE/api?action=comments&kind=article&target=1" | grep -q '待审内容'; then echo "先审后发却直接可见"; exit 1; fi
curl -s -b "$J" "$BASE/api?action=comments-admin&status=pending" | grep -q "待审内容" || { echo "先审后发未进待审"; exit 1; }
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"comment_provider":"","comment_moderation":"","comment_scope":""}' "$BASE/api?action=save-settings" >/dev/null

# Gravatar 哈希：带邮箱的评论返回 email_hash（且不回传明文邮箱）
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"kind":"article","target":1,"author":"头像君","email":"someone@example.com","content":"看头像"}' "$BASE/api?action=comment" >/dev/null
curl -s "$BASE/api?action=comments&kind=article&target=1" | python3 -c '
import sys, json
cs = [c for c in json.load(sys.stdin)["comments"] if c["content"] == "看头像"]
assert cs and len(cs[0]["email_hash"]) == 32 and "email" not in cs[0], "email_hash 缺失或邮箱泄露"
print("gravatar hash ok")' || exit 1


# 点赞：切换 + 计数 + 访客去重
LTK=$(python3 -c "print('a'*32)")
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d "{\"id\":1,\"token\":\"$LTK\"}" "$BASE/api?action=like" | grep -q '"liked":true'
curl -s "$BASE/api?action=likes&token=$LTK" | grep -q '\[1\]'
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d "{\"id\":1,\"token\":\"$LTK\"}" "$BASE/api?action=like" | grep -q '"liked":false'
curl -s "$BASE/api?action=posts" | python3 -c 'import sys,json;ps=json.load(sys.stdin)["posts"];assert any(p.get("like_count")==0 for p in ps)' || { echo "点赞计数异常"; exit 1; }

# 安全：公开 state 不得泄露 SMTP / Akismet 凭据
if curl -s "$BASE/api?action=state" | grep -qE 'smtp_user|akismet_key|smtp_host|netlify_token'; then echo "公开 state 泄露凭据"; exit 1; fi

# 一键部署：状态接口未登录须 401；已登录但未配凭据须 422 且不派生任务
curl -s -o /dev/null -w '%{http_code}' "$BASE/api?action=deploy-status" | grep -q 401 || { echo "deploy-status 未登录可读"; exit 1; }
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{}' "$BASE/api?action=netlify-deploy" | grep -q '请先填写并保存'; then :; else echo "无凭据部署未被拒绝"; exit 1; fi

# 自动部署：开关与防抖校验 + 合法保存（保存后须出现在 state）+ 非法拒绝
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"auto_deploy":"1","auto_deploy_debounce":10}' "$BASE/api?action=save-settings" | grep -q '"ok":true'
curl -s -b "$J" "$BASE/api?action=state" | grep -q '"auto_deploy":true' || { echo "自动部署开关未生效"; exit 1; }
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"auto_deploy":"2"}' "$BASE/api?action=save-settings" | grep -q '取值无效'; then :; else echo "自动部署非法值未被拒绝"; exit 1; fi
if curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"auto_deploy_debounce":9999}' "$BASE/api?action=save-settings" | grep -q '之间'; then :; else echo "防抖上限未生效"; exit 1; fi
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  -d '{"auto_deploy":"0"}' "$BASE/api?action=save-settings" >/dev/null

# 备份往返
curl -s -b "$J" "$BASE/api?action=backup-export" > /tmp/mkblog-smoke-backup.json
curl -s -b "$J" -H "X-CSRF-Token: $CSRF" -H 'Content-Type: application/json' \
  --data-binary @/tmp/mkblog-smoke-backup.json "$BASE/api?action=backup-import" | grep -q '"ok":true'
rm -f /tmp/mkblog-smoke-backup.json

echo "api.sh PASSED"
