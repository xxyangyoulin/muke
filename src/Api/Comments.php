<?php
declare(strict_types=1);

// 公开：已审评论列表（GET）与提交（POST，蜜罐 + 限流，见 src/Comments.php）
function api_comments(PDO $db, bool $admin): never {
    $kind = (string)($_GET['kind'] ?? '');
    $target = (int)($_GET['target'] ?? 0);
    json_response(['comments' => comments_of($db, $kind, $target)]);
}

function api_comment(PDO $db, array $input, string $action, bool $admin): never {
    [$ok, $msg] = comment_submit($db, $input);
    if (!$ok) json_response(['error' => $msg], 422);
    // 邮件通知（未配置 SMTP 时静默跳过）：管理员全量、回复对象（其已审+本条放行）
    $cid = (int)$db->lastInsertId();
    if ($cid > 0) {
        $stmt = $db->prepare('SELECT * FROM comments WHERE id = ?');
        $stmt->execute([$cid]);
        if ($c = $stmt->fetch(PDO::FETCH_ASSOC)) {
            $c['target_label'] = $db->query('SELECT title FROM articles WHERE id = ' . (int)$c['target_id'])->fetchColumn() ?: ('#' . $c['target_id']);
            $c['url'] = site_absolute_url($c['kind'] === 'article' ? '/article/' . $c['target_id'] : '/#post-' . $c['target_id']);
            if ($c['status'] !== 'spam') mail_notify_admin($c);
            if ($c['reply_to']) {
                $stmt2 = $db->prepare('SELECT author, email, status FROM comments WHERE id = ?');
                $stmt2->execute([(int)$c['reply_to']]);
                if ($parent = $stmt2->fetch(PDO::FETCH_ASSOC)) {
                    $c['author'] = $c['author'];
                    mail_notify_reply($c, $parent);
                }
            }
        }
    }
    plugin_fire('comment.created', ['id' => $cid, 'comment' => $c]);
    // pending 时不透露"被待审"（防垃圾方探测），统一提示成功
    json_response(['ok' => true]);
}

// 管理：按状态列评论（all / pending / spam），带目标摘要
function api_comments_admin(PDO $db, bool $admin): never {
    if (!$admin) json_response(['error' => '请先登录管理后台'], 401);
    $status = (string)($_GET['status'] ?? 'all');
    if (!in_array($status, ['all', 'approved', 'pending', 'spam'], true)) $status = 'all';
    $where = $status === 'all' ? '' : ' WHERE status = ' . $db->quote($status);
    $rows = $db->query('SELECT * FROM comments' . $where . ' ORDER BY id DESC LIMIT 500')->fetchAll(PDO::FETCH_ASSOC);
    $titles = [];
    foreach ($db->query('SELECT id, title FROM articles') as $a) $titles['article:' . $a['id']] = $a['title'];
    foreach ($db->query('SELECT id, content FROM posts') as $p) $titles['post:' . $p['id']] = mb_substr($p['content'], 0, 30);
    foreach ($rows as &$r) {
        $r['target_label'] = $titles[$r['kind'] . ':' . $r['target_id']] ?? ('#' . $r['target_id']);
        unset($r['email'], $r['ip_hash']); // 列表不回传敏感字段
    }
    unset($r);
    $counts = ['all' => 0, 'approved' => 0, 'pending' => 0, 'spam' => 0];
    foreach ($db->query('SELECT status, COUNT(*) n FROM comments GROUP BY status') as $c) $counts[$c['status']] = (int)$c['n'];
    $counts['all'] = array_sum(array_diff_key($counts, ['all' => 0]));
    json_response(['comments' => $rows, 'counts' => $counts]);
}

// 管理：标垃圾 / 恢复 / 删除（同 IP 哈希命中垃圾后，新评论自动进待审）
function api_moderate_comment(PDO $db, array $input, string $action, bool $admin): never {
    $id = (int)($input['id'] ?? 0);
    $op = (string)($input['op'] ?? '');
    if (!in_array($op, ['spam', 'approve', 'delete'], true)) json_response(['error' => '操作无效'], 422);
    $stmt = $db->prepare('SELECT id FROM comments WHERE id = ?');
    $stmt->execute([$id]);
    if (!$stmt->fetchColumn()) json_response(['error' => '评论不存在'], 404);
    if ($op === 'delete') {
        $db->prepare('DELETE FROM comments WHERE id = ? OR reply_to = ?')->execute([$id, $id]);
    } else {
        $db->prepare('UPDATE comments SET status = ? WHERE id = ?')->execute([$op === 'spam' ? 'spam' : 'approved', $id]);
    }
    plugin_fire('comment.moderated', ['id' => $id, 'op' => $op]);
    json_response(['ok' => true]);
}
