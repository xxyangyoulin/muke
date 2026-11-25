<?php
declare(strict_types=1);

function api_save_post(PDO $db, array $input, string $action, bool $admin): never {
    $uploaded = [];
    $removed = [];
    try {
        $content = trim((string)($input['content'] ?? ''));
        $maxChars = (int)opt('post_max_chars');
    if (mb_strlen($content) > $maxChars) json_response(['error' => "动态内容最多 {$maxChars} 字"], 422);
        $existing = null;
        if (!empty($input['id'])) {
            $stmt = $db->prepare('SELECT * FROM posts WHERE id = ?');
            $stmt->execute([(int)$input['id']]);
            $existing = $stmt->fetch(PDO::FETCH_ASSOC);
            if (!$existing) json_response(['error' => '动态已不存在'], 404);
        }
        $images = [];
        if ($existing) {
            $keep = $input['keep'] ?? [];
            if (is_string($keep)) $keep = json_decode($keep, true) ?: [];
            $refs = json_decode($existing['images'] ?? '[]', true) ?: [];
            $images = array_values(array_intersect($refs, array_values((array)$keep)));
            $removed = array_diff($refs, $images);
        }
        $count = count($images);
        foreach (array_keys($_FILES['images']['error'] ?? []) as $index) $count++;
        $maxImages = (int)opt('post_max_images');
        if ($count > $maxImages) json_response(['error' => "每条动态最多 {$maxImages} 张图片"], 422);
        foreach (array_keys($_FILES['images']['error'] ?? []) as $index) $uploaded[] = save_uploaded_image($_FILES['images'], (int)$index);
        $images = array_merge($images, $uploaded);
        if ($content === '' && !$images) json_response(['error' => '写点什么，或者配一张图'], 422);
        $now = date(DATE_ATOM);
        $encoded = json_encode($images);
        if ($existing) {
            $stmt = $db->prepare('UPDATE posts SET content = ?, images = ?, updated_at = ? WHERE id = ?');
            $stmt->execute([$content, $encoded, $now, $existing['id']]);
        } else {
            $stmt = $db->prepare('INSERT INTO posts (content, images, created_at, updated_at) VALUES (?, ?, ?, ?)');
            $stmt->execute([$content, $encoded, $now, $now]);
            $savedPostId = (int)$db->lastInsertId();
        }
        $uploaded = [];
        foreach ($removed as $ref) delete_image_file($ref);
        plugin_fire('post.saved', ['id' => $savedPostId ?? (int)$input['id'], 'content' => $content]);
        if (!isset($existing)) auto_deploy_touch($db); // 新动态：触发自动部署防抖计时（编辑不触发）
        json_response(['ok' => true]);
    } catch (RuntimeException $e) {
        foreach ($uploaded as $ref) delete_image_file($ref);
        json_response(['error' => $e->getMessage()], 422);
    }
}

function api_pin_post(PDO $db, array $input, string $action, bool $admin): never {
    $id = filter_var($input['id'] ?? null, FILTER_VALIDATE_INT);
    if (!$id || !in_array($input['pinned'] ?? null, [0, 1], true)) json_response(['error' => '置顶参数无效'], 422);
    $stmt = $db->prepare('UPDATE posts SET pinned = ? WHERE id = ?');
    $stmt->execute([$input['pinned'], $id]);
    if (!$stmt->rowCount()) json_response(['error' => '动态不存在'], 404);
    auto_deploy_touch($db);
    json_response(['ok' => true]);
}

function api_delete_post(PDO $db, array $input, string $action, bool $admin): never {
    $id = (int)($input['id'] ?? 0);
    $stmt = $db->prepare('SELECT images, source, source_key FROM posts WHERE id = ?');
    $stmt->execute([$id]);
    $row = $stmt->fetch(PDO::FETCH_ASSOC);
    $db->beginTransaction();
    try {
    if ($row && $row['source_key'] !== '') {
        // 墓碑：用户删掉的插件动态，后续同步永不再出现
        $db->prepare('INSERT OR IGNORE INTO plugin_tombstones (key, plugin, deleted_at) VALUES (?, ?, ?)')->execute([$row['source_key'], $row['source'], date(DATE_ATOM)]);
    }
    $db->prepare('DELETE FROM posts WHERE id = ?')->execute([$id]);
    $db->prepare("DELETE FROM comments WHERE kind = 'post' AND target_id = ?")->execute([$id]);
    $db->prepare('DELETE FROM post_likes WHERE post_id = ?')->execute([$id]);
    $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    if ($row && $row['images']) foreach (json_decode($row['images'], true) ?: [] as $ref) delete_image_file($ref);
    plugin_fire('post.deleted', ['id' => $id, 'source' => $row['source'] ?? '', 'source_key' => $row['source_key'] ?? '']);
    // 插件删除后置钩子（如壁纸把对应记录标记为未发布）
    if ($row && ($row['source'] ?? '') !== '') {
        $plugin = plugin_list()[$row['source']] ?? null;
        if ($plugin && !empty(plugin_config($db, $row['source'])['_enabled']) && !empty($plugin['after_post_delete']) && is_callable($plugin['after_post_delete'])) ($plugin['after_post_delete'])($db, $row);
    }
    json_response(['ok' => true]);
}

function api_render(PDO $db, array $input, string $action, bool $admin): never {
    $content = (string)($input['content'] ?? '');
    $format = ($input['format'] ?? 'md') === 'html' ? 'html' : 'md';
    if (strlen($content) > (int)opt('article_max_bytes')) json_response(['error' => '内容过长'], 422);
    json_response(['html' => render_article(['content' => $content, 'format' => $format])]);
}

function api_upload_image(PDO $db, array $input, string $action, bool $admin): never {
    try {
        $file = $_FILES['image'] ?? null;
        if (!$file || ($file['error'] ?? UPLOAD_ERR_NO_FILE) !== UPLOAD_ERR_OK) json_response(['error' => '请选择图片'], 422);
        $ref = save_uploaded_image(['error' => [$file['error']], 'size' => [$file['size']], 'tmp_name' => [$file['tmp_name']]], 0);
        json_response(['ok' => true, 'url' => '/image?upload=' . substr($ref, 3)]);
    } catch (RuntimeException $e) {
        json_response(['error' => $e->getMessage()], 422);
    }
}

function api_save_article(PDO $db, array $input, string $action, bool $admin): never {
    $title = trim((string)($input['title'] ?? ''));
    $content = (string)($input['content'] ?? '');
    $format = ($input['format'] ?? 'md') === 'html' ? 'html' : 'md';
    $tags = normalize_tags((string)($input['tags'] ?? ''));
    $draft = !empty($input['draft']) ? 1 : 0;
    if (mb_strlen($title) < 1 || mb_strlen($title) > 80) json_response(['error' => '标题需 1–80 字'], 422);
    if (trim($content) === '' || strlen($content) > (int)opt('article_max_bytes')) json_response(['error' => '正文不能为空，且不超过大小上限'], 422);
    $refs = article_image_refs($content);
    $db->beginTransaction();
    try {
    $db->exec('UPDATE articles SET id = id WHERE 0');
    $existing = null;
    if (!empty($input['id'])) {
        $stmt = $db->prepare('SELECT * FROM articles WHERE id = ?');
        $stmt->execute([(int)$input['id']]);
        $existing = $stmt->fetch(PDO::FETCH_ASSOC);
        if (!$existing) { $db->rollBack(); json_response(['error' => '文章已不存在'], 404); }
        $format = $existing['format']; // 编辑沿用创建时的格式，避免互相转换失真
    }
    $encoded = json_encode($refs);
    if ($existing) {
        update_article_with_revision($db, $existing, ['title' => $title, 'content' => $content, 'format' => $format, 'tags' => $tags, 'draft' => $draft, 'images' => $encoded]);
    } else {
        $now = date(DATE_ATOM);
        $aid = null; // 占位：下面 insert 后取 lastInsertId
        $stmt = $db->prepare('INSERT INTO articles (title, content, format, tags, draft, images, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)');
        $stmt->execute([$title, $content, $format, $tags, $draft, $encoded, $now, $now]);
        $aid = (int)$db->lastInsertId();
    }
    $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    plugin_fire('article.saved', ['id' => $aid ?? (int)$existing['id'], 'title' => $title, 'draft' => $draft]);
    json_response(['ok' => true]);
}

function api_delete_article(PDO $db, array $input, string $action, bool $admin): never {
    $id = (int)($input['id'] ?? 0);
    $db->beginTransaction();
    try {
    $db->exec('UPDATE articles SET id = id WHERE 0');
    $stmt = $db->prepare('SELECT images FROM articles WHERE id = ? UNION ALL SELECT images FROM article_revisions WHERE article_id = ?');
    $stmt->execute([$id, $id]);
    $refs = [];
    foreach ($stmt->fetchAll(PDO::FETCH_COLUMN) as $images) $refs = array_merge($refs, json_decode($images, true) ?: []);
    $db->prepare('DELETE FROM article_revisions WHERE article_id = ?')->execute([$id]);
    $db->prepare('DELETE FROM articles WHERE id = ?')->execute([$id]);
    $db->prepare("DELETE FROM comments WHERE kind = 'article' AND target_id = ?")->execute([$id]);
    $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    foreach (array_unique($refs) as $name) delete_image_file('up:' . $name);
    plugin_fire('article.deleted', ['id' => $id]);
    json_response(['ok' => true]);
}


/** 点赞 / 取消点赞（访客令牌去重，幂等切换） */
/** 点赞限速：同 IP 哈希 60 秒内 30 次（文件计数，防刷表） */
function like_throttled(string $ipHash): bool {
    $file = DATA_DIR . '/like.throttle.json';
    $now = time();
    $key = hash('sha256', $ipHash . '|like');
    $data = is_file($file) ? json_decode((string)file_get_contents($file), true) ?: [] : [];
    $data = array_values(array_filter($data, fn($e) => $now - ($e['t'] ?? 0) < 60));
    $mine = array_filter($data, fn($e) => ($e['k'] ?? '') === $key);
    if (count($mine) >= 30) return true;
    $data[] = ['k' => $key, 't' => $now];
    file_put_contents($file, json_encode(array_slice($data, -500)), LOCK_EX);
    return false;
}

function api_like(PDO $db, array $input, string $action, bool $admin): never {
    $id = (int)($input['id'] ?? 0);
    $token = (string)($input['token'] ?? '');
    if (!preg_match('/^[a-f0-9]{16,64}$/', $token)) json_response(['error' => '访客标识无效'], 422);
    if (like_throttled(comment_ip_hash())) json_response(['error' => '操作太频繁了，请稍后再试'], 429);
    $stmt = $db->prepare('SELECT COUNT(*) FROM posts WHERE id = ?');
    $stmt->execute([$id]);
    if (!$stmt->fetchColumn()) json_response(['error' => '动态不存在'], 404);
    $stmt = $db->prepare('SELECT COUNT(*) FROM post_likes WHERE post_id = ? AND token = ?');
    $stmt->execute([$id, $token]);
    if ((int)$stmt->fetchColumn()) {
        $db->prepare('DELETE FROM post_likes WHERE post_id = ? AND token = ?')->execute([$id, $token]);
        $liked = false;
    } else {
        $db->prepare('INSERT OR IGNORE INTO post_likes (post_id, token, created_at) VALUES (?, ?, ?)')->execute([$id, $token, date(DATE_ATOM)]);
        $liked = true;
    }
    $stmt = $db->prepare('SELECT COUNT(*) FROM post_likes WHERE post_id = ?');
    $stmt->execute([$id]);
    json_response(['ok' => true, 'liked' => $liked, 'count' => (int)$stmt->fetchColumn()]);
}
