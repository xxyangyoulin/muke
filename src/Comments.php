<?php
declare(strict_types=1);

// 评论域：匿名评论（昵称 + 可选邮箱/网址），先发后审；ip_hash 只存哈希不留原值。
// 反垃圾三板斧：蜜罐字段（API 层）+ 同 IP 60 秒限流 + 垃圾 IP 哈希联动进待审。

const COMMENT_KINDS = ['article', 'post'];

function comment_ip_hash(): string {
    $ip = $_SERVER['REMOTE_ADDR'] ?? '';
    return hash('sha256', $ip . '|' . DATA_DIR); // 每安装独立盐，不存原始 IP
}

/** 目标的已审评论（时间正序，会话式阅读；附回复对象昵称） */
function comments_of(PDO $db, string $kind, int $targetId): array {
    if (!in_array($kind, COMMENT_KINDS, true)) return [];
    $stmt = $db->prepare('SELECT id, author, email, site, content, created_at, reply_to FROM comments WHERE kind = ? AND target_id = ? AND status = \'approved\' ORDER BY id ASC');
    $stmt->execute([$kind, $targetId]);
    $rows = $stmt->fetchAll(PDO::FETCH_ASSOC);
    $names = [];
    foreach ($rows as $r) $names[(int)$r['id']] = $r['author'];
    foreach ($rows as &$r) {
        $r['reply_to'] = $r['reply_to'] !== null ? (int)$r['reply_to'] : null;
        $r['reply_name'] = $r['reply_to'] !== null ? ($names[$r['reply_to']] ?? null) : null;
        $r['email_hash'] = $r['email'] !== '' ? md5(strtolower(trim($r['email']))) : '';
        unset($r['email']); // 邮箱不外发，仅保留 Gravatar 哈希
    }
    unset($r);
    return $rows;
}

/** 批量评论数（approved），键 "kind:id" => int */
function comment_counts(PDO $db, ?string $kind = null, array $ids = []): array {
    $counts = [];
    if ($kind !== null && !$ids) return [];
    $sql = "SELECT kind, target_id, COUNT(*) n FROM comments WHERE status = 'approved'";
    $params = [];
    if ($kind !== null) {
        $sql .= ' AND kind = ? AND target_id IN (' . implode(',', array_fill(0, count($ids), '?')) . ')';
        $params = [$kind, ...$ids];
    }
    $stmt = $db->prepare($sql . ' GROUP BY kind, target_id');
    $stmt->execute($params);
    foreach ($stmt as $row) {
        $counts[$row['kind'] . ':' . $row['target_id']] = (int)$row['n'];
    }
    return $counts;
}

/** 该 IP 哈希最近是否已被标垃圾（新评论自动进 pending） */
function comment_ip_flagged(PDO $db, string $ipHash): bool {
    $stmt = $db->prepare('SELECT COUNT(*) FROM comments WHERE ip_hash = ? AND status = \'spam\'');
    $stmt->execute([$ipHash]);
    return (int)$stmt->fetchColumn() > 0;
}

/** 同 IP 频控窗口内是否已提交过（窗口 comment_rate_seconds 可配，默认 60 秒，0=不限） */
function comment_rate_limited(PDO $db, string $ipHash): bool {
    $window = (int)opt('comment_rate_seconds');
    if ($window <= 0) return false;
    $stmt = $db->prepare('SELECT COUNT(*) FROM comments WHERE ip_hash = ? AND created_at > ?');
    $stmt->execute([$ipHash, date(DATE_ATOM, time() - $window)]);
    return (int)$stmt->fetchColumn() > 0;
}

/** 提交评论：通过校验返回 [ok, error]；kind+target 需存在 */
function comment_submit(PDO $db, array $in): array {
    $kind = (string)($in['kind'] ?? '');
    $targetId = (int)($in['target'] ?? 0);
    if (!comment_enabled()) return [false, '评论已关闭'];
    if (!in_array($kind, COMMENT_KINDS, true)) return [false, '评论目标无效'];
    if (comment_article_only() && $kind !== 'article') return [false, '此处已关闭评论'];
    if ($kind === 'article') {
        $stmt = $db->prepare('SELECT COUNT(*) FROM articles WHERE id = ? AND draft = 0');
        $stmt->execute([$targetId]);
        if (!(int)$stmt->fetchColumn()) return [false, '文章不存在'];
    } else {
        $stmt = $db->prepare('SELECT COUNT(*) FROM posts WHERE id = ?');
        $stmt->execute([$targetId]);
        if (!(int)$stmt->fetchColumn()) return [false, '动态不存在'];
    }
    // 蜜罐：机器人会填隐藏字段 —— 假装成功但不入库
    if (trim((string)($in['website_url'] ?? '')) !== '') return [true, ''];
    $author = trim((string)($in['author'] ?? ''));
    $email = trim((string)($in['email'] ?? ''));
    $site = trim((string)($in['site'] ?? ''));
    $content = trim((string)($in['content'] ?? ''));
    if ($author === '' || mb_strlen($author) > 40) return [false, '昵称必填且不超过 40 字'];
    if ($content === '' || mb_strlen($content) > 1000) return [false, '评论内容必填且不超过 1000 字'];
    if ($email !== '' && (!filter_var($email, FILTER_VALIDATE_EMAIL) || mb_strlen($email) > 100)) return [false, '邮箱格式不正确'];
    if ($site !== '') {
        if (mb_strlen($site) > 200 || !preg_match('#^https?://#', $site)) return [false, '网址需以 http(s):// 开头'];
        $site = htmlspecialchars($site, ENT_QUOTES, 'UTF-8');
    }
    $replyTo = (int)($in['reply_to'] ?? 0);
    if ($replyTo > 0) {
        $stmt = $db->prepare('SELECT COUNT(*) FROM comments WHERE id = ? AND kind = ? AND target_id = ? AND status = \'approved\'');
        $stmt->execute([$replyTo, $kind, $targetId]);
        if (!(int)$stmt->fetchColumn()) $replyTo = 0; // 回复目标无效则降级为普通评论
    }
    $ipHash = comment_ip_hash();
    if (comment_rate_limited($db, $ipHash)) return [false, '评论太频繁了，稍等一分钟再试试'];
    $status = 'approved';
    if (comment_pre_moderation() || comment_ip_flagged($db, $ipHash)) $status = 'pending';
    // Akismet（可选）：配了 akismet_key 才检查，判垃圾直接进 spam 且对提交方假成功
    $akismet = trim((string)opt('akismet_key'));
    if ($akismet !== '') {
        $verdict = http_post_form("https://{$akismet}.rest.akismet.com/1.1/comment-check", [
            'key' => $akismet, 'blog' => site_absolute_url('/'),
            'user_ip' => $_SERVER['REMOTE_ADDR'] ?? '', 'user_agent' => substr((string)($_SERVER['HTTP_USER_AGENT'] ?? ''), 0, 200),
            'comment_type' => 'comment', 'comment_author' => $author, 'comment_author_email' => $email, 'comment_content' => $content,
        ]);
        if (trim($verdict) === 'true') $status = 'spam';
    }
    $db->prepare('INSERT INTO comments (kind, target_id, author, email, site, content, created_at, status, reply_to, ip_hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
        ->execute([$kind, $targetId, $author, $email, $site, $content, date(DATE_ATOM), $status, $replyTo ?: null, $ipHash]);
    return [true, $status];
}

// ---- 第三方评论接入（giscus / Waline / Twikoo / 自定义 iframe）----

/** 已配置的评论提供方：'' = 原生，'off' = 关闭，其余为第三方 */
function comment_provider(): string {
    $p = (string)opt('comment_provider');
    if ($p === 'off') return 'off';
    return in_array($p, ['giscus', 'waline', 'twikoo', 'custom'], true) ? $p : '';
}

/** 评论总开关（off 时动态无入口、API 拒绝、静态不烘焙不嵌入） */
function comment_enabled(): bool {
    return comment_provider() !== 'off';
}

/** 审核策略：'' = 先发后审（默认），'pre' = 先审后发（全部进待审） */
function comment_pre_moderation(): bool {
    return (string)opt('comment_moderation') === 'pre';
}

/** 评论范围：'' = 文章+动态，'article' = 仅文章 */
function comment_article_only(): bool {
    return (string)opt('comment_scope') === 'article';
}

/** 嵌入地址（custom = iframe src；waline = serverURL；twikoo = envId/部署地址） */
function comment_embed_url(): string {
    return trim((string)opt('comment_embed_url'));
}

/** 需要追加到全站 CSP 的 frame-src 源（giscus/custom 的外部 iframe；waline/twikoo 走同源 embed 页不需要） */
function comment_csp_extra(): string {
    $p = comment_provider();
    if ($p === 'giscus') return "; frame-src 'self' https://giscus.app";
    if ($p !== 'custom') return '';
    $url = comment_embed_url();
    if (!preg_match('#^https?://#', $url)) return '';
    $origin = parse_url($url, PHP_URL_SCHEME) . '://' . parse_url($url, PHP_URL_HOST) . (parse_url($url, PHP_URL_PORT) ? ':' . parse_url($url, PHP_URL_PORT) : '');
    return "; frame-src 'self' " . $origin;
}

/** 同源嵌入页（waline/twikoo 用）：动态站由 index.php 路由输出，静态导出写成 dist/embed/comments/index.html */
function comments_embed_html(): string {
    $p = comment_provider();
    $url = h(comment_embed_url());
    $css = 'body{margin:0;background:transparent;font-family:inherit}';
    return '<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">'
        . '<link rel="stylesheet" href="/assets/embed.css"><title>评论</title></head>'
        . '<body><div id="embed-root"></div>'
        . '<script src="/assets/embed-init.js" data-provider="' . h($p) . '" data-url="' . $url . '" defer></script></body></html>';
}

/** 前端 iframe 片段（文章页/嵌入第三方时使用） */
function comments_iframe_html(string $kind, int $id): string {
    if (!comment_enabled()) return '';
    $p = comment_provider();
    if ($p === 'giscus') {
        if (!(string)opt('giscus_repo') || !(string)opt('giscus_repo_id') || !(string)opt('giscus_category_id')) return '';
        $query = http_build_query(['repo' => (string)opt('giscus_repo'), 'repoId' => (string)opt('giscus_repo_id'), 'categoryId' => (string)opt('giscus_category_id'), 'term' => $kind . ':' . $id, 'strict' => '1', 'theme' => 'light', 'reactionsEnabled' => '1', 'inputPosition' => 'top'], '', '&', PHP_QUERY_RFC3986);
        return '<div class="panel comments-panel embed-host"><iframe class="comment-embed" src="https://giscus.app/zh-CN/widget?' . h($query) . '" title="giscus 评论" loading="lazy" allow="clipboard-write"></iframe></div>';
    }
    if ($p === 'custom') {
        $src = comment_embed_url();
        return preg_match('#^https?://#', $src) ? '<div class="panel comments-panel embed-host"><iframe class="comment-embed" src="' . h($src) . '" title="评论" loading="lazy" allow="clipboard-write"></iframe></div>' : '';
    }
    if ($p === 'waline' || $p === 'twikoo') {
        return '<div class="panel comments-panel embed-host"><iframe class="comment-embed" src="/embed/comments/" title="评论" loading="lazy"></iframe></div>';
    }
    return '';
}

/** 静态导出烘焙：已审评论只读 HTML（无 JS 可读） */
function comments_readonly_html(PDO $db, string $kind, int $targetId): string {
    if (!comment_enabled()) return '';
    $rows = comments_of($db, $kind, $targetId);
    if (!$rows) return '';
    $e = fn($x) => htmlspecialchars((string)$x, ENT_QUOTES, 'UTF-8');
    $html = '<section class="comments-panel comments-baked"><div class="section-heading"><h3>评论 <small>' . count($rows) . ' 条</small></h3></div>';
    foreach ($rows as $c) {
        $html .= '<div class="comment-row"><span class="comment-avatar">' . $e(mb_substr($c['author'], 0, 1)) . '</span><div class="comment-main"><div class="comment-head"><strong>' . $e($c['author']) . '</strong>'
            . (!empty($c['reply_name']) ? '<span class="comment-reply-at">回复 @' . $e($c['reply_name']) . '</span>' : '')
            . '<time>' . $e(date_short_label($c['created_at'])) . '</time></div><p class="comment-content">' . $e($c['content']) . '</p></div></div>';
    }
    return $html . '</section>';
}

function giscus_export_counts(array $terms, ?callable $checkCancelled = null): array {
    $queue = array_map(fn($term) => ['term' => $term, 'after' => ''], array_values(array_unique($terms)));
    $counts = [];
    while ($queue) {
        if ($checkCancelled) $checkCancelled();
        $multi = curl_multi_init();
        $handles = [];
        try {
            foreach (array_splice($queue, 0, 4) as $job) {
                $query = http_build_query(['repo' => (string)opt('giscus_repo'), 'term' => $job['term'], 'strict' => 'true', 'first' => 50, 'after' => $job['after']]);
                $ch = curl_init('https://giscus.app/api/discussions?' . $query);
                curl_setopt_array($ch, [CURLOPT_RETURNTRANSFER => true, CURLOPT_TIMEOUT => 30, CURLOPT_CONNECTTIMEOUT => 10, CURLOPT_PROXY => (string)opt('http_proxy'), CURLOPT_HTTPHEADER => ['Accept: application/json'], CURLOPT_USERAGENT => 'MUKE static export']);
                curl_multi_add_handle($multi, $ch);
                $handles[] = [$ch, $job];
            }
            do {
                if ($checkCancelled) $checkCancelled();
                $code = curl_multi_exec($multi, $running);
                if ($code !== CURLM_OK) throw new RuntimeException('giscus 统计查询失败');
                if ($running) curl_multi_select($multi, 0.2);
            } while ($running);
            foreach ($handles as [$ch, $job]) {
                $status = (int)curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
                $data = json_decode((string)curl_multi_getcontent($ch), true);
                $term = $job['term'];
                if ($status === 404 && ($data['error'] ?? '') === 'Discussion not found' && $job['after'] === '') {
                    $counts[$term] = ['comments' => 0, 'likes' => 0];
                    continue;
                }
                $discussion = $data['discussion'] ?? [];
                $comments = $discussion['totalCommentCount'] ?? null;
                $replies = $discussion['totalReplyCount'] ?? null;
                $likes = $discussion['reactions']['HEART']['count'] ?? null;
                if ($status !== 200 || !is_int($comments) || $comments < 0 || !is_int($replies) || $replies < 0 || !is_int($likes) || $likes < 0) {
                    throw new RuntimeException('giscus 统计同步失败（' . $term . '，HTTP ' . $status . '），请稍后重试');
                }
                if ($job['after'] === '') $counts[$term] = ['comments' => $comments, 'likes' => $likes];
                $counts[$term]['comments'] += $replies;
                if (!empty($discussion['pageInfo']['hasNextPage'])) {
                    $cursor = $discussion['pageInfo']['endCursor'] ?? '';
                    if (!is_string($cursor) || $cursor === '' || $cursor === $job['after']) throw new RuntimeException('giscus 统计分页游标无效');
                    $queue[] = ['term' => $term, 'after' => $cursor];
                }
            }
        } finally {
            foreach ($handles as [$ch]) { curl_multi_remove_handle($multi, $ch); curl_close($ch); }
            curl_multi_close($multi);
        }
    }
    return $counts;
}
