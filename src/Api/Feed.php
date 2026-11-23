<?php
declare(strict_types=1);

function api_state(PDO $db, bool $admin): never {
    dispatch_plugin_sync($db);
    $settings = settings($db);
    $avatarFile = DATA_DIR . '/avatar.webp';
    $coverFile = DATA_DIR . '/cover.webp';
    $plugins = public_plugins();
    $state = ['admin' => $admin, 'setup' => !(bool)$db->query('SELECT COUNT(*) FROM users')->fetchColumn(), 'csrf' => $_SESSION['csrf'], 'profile' => ['name' => $settings['name'] ?? config('site_name'), 'bio' => $settings['bio'] ?? (string)config('profile_bio'), 'handle' => $settings['handle'] ?? (string)config('profile_handle'), 'avatar' => is_file($avatarFile) ? '/image?avatar=1&v=' . filemtime($avatarFile) : null, 'cover' => is_file($coverFile) ? '/image?cover=1&v=' . filemtime($coverFile) : null], 'plugins' => $plugins, 'options' => public_options()];
    if ($admin) {
        $state['version'] = MKBLOG_VERSION;
        $state['netlify'] = ['site' => $settings['netlify_site'] ?? '', 'has_token' => isset($settings['netlify_token']), 'auto_deploy' => opt('auto_deploy') === '1', 'auto_deploy_debounce' => (int)opt('auto_deploy_debounce') ?: 10];
        // SMTP / Akismet 凭据仅管理员可见（smtp_pass 永不下发，留空=保持已存）
        $state['comment_settings'] = ['smtp_host' => (string)opt('smtp_host'), 'smtp_port' => (int)opt('smtp_port'), 'smtp_user' => (string)opt('smtp_user'), 'notify_email' => (string)opt('notify_email'), 'akismet_key' => (string)opt('akismet_key')];
    }
    json_response($state);
}

function api_posts(PDO $db, bool $admin): never {
    $limit = max(1, min(100, (int)($_GET['limit'] ?? 30)));
    $query = mb_substr(trim((string)($_GET['q'] ?? '')), 0, 200);
    $where = $query === '' ? '1 = 1' : 'p.content LIKE ?';
    $params = $query === '' ? [] : ['%' . $query . '%'];
    $total = $db->prepare('SELECT COUNT(*) FROM posts p WHERE ' . $where);
    $total->execute($params);
    $count = (int)$total->fetchColumn();
    if (!empty($_GET['cursor'])) {
        $cursor = json_decode(base64_decode((string)$_GET['cursor'], true) ?: '', true);
        if (!is_array($cursor) || !array_is_list($cursor) || count($cursor) !== 2 || !is_string($cursor[0]) || !is_int($cursor[1])) json_response(['error' => '分页游标无效'], 422);
        $where .= ' AND (p.created_at, p.id) < (?, ?)';
        array_push($params, $cursor[0], $cursor[1]);
    }
    $stmt = $db->prepare('SELECT p.*, (SELECT COUNT(*) FROM post_likes l WHERE l.post_id = p.id) AS like_count FROM posts p WHERE ' . $where . ' ORDER BY p.created_at DESC, p.id DESC LIMIT ' . ($limit + 1));
    $stmt->execute($params);
    $posts = $stmt->fetchAll(PDO::FETCH_ASSOC);
    $more = count($posts) > $limit;
    if ($more) array_pop($posts);
    $last = end($posts);
    $commentCounts = comment_counts($db, 'post', array_column($posts, 'id'));
    foreach ($posts as &$post) { $post['images'] = post_images($db, $post['images'] ?? '[]'); $post['meta'] = post_meta($post); $post['comment_count'] = $commentCounts['post:' . $post['id']] ?? 0; }
    unset($post);
    json_response(['posts' => $posts, 'total' => $count, 'next_cursor' => $more ? base64_encode(json_encode([$last['created_at'], (int)$last['id']])) : null]);
}

function api_articles(PDO $db, bool $admin): never {
    if (isset($_GET['id'])) {
        $stmt = $db->prepare('SELECT * FROM articles WHERE id = ?' . ($admin ? '' : ' AND draft = 0'));
        $stmt->execute([(int)$_GET['id']]);
        $article = $stmt->fetch(PDO::FETCH_ASSOC);
        if (!$article) json_response(['error' => '文章不存在'], 404);
        $article['html'] = render_article($article);
        $article['comment_count'] = comment_counts($db, 'article', [$article['id']])['article:' . $article['id']] ?? 0;
        json_response(['article' => $article]);
    }
    $draft = !empty($_GET['draft']) ? 1 : 0;
    if ($draft && !$admin) json_response(['error' => '请先登录管理后台'], 401);
    $limit = max(1, min(100, (int)($_GET['limit'] ?? 30)));
    $where = 'draft = ?';
    $params = [$draft];
    $query = mb_substr(trim((string)($_GET['q'] ?? '')), 0, 200);
    if ($query !== '') {
        $where .= ' AND (title LIKE ? OR content LIKE ? OR tags LIKE ?)';
        array_push($params, '%' . $query . '%', '%' . $query . '%', '%' . $query . '%');
    }
    $tag = mb_substr((string)($_GET['tag'] ?? ''), 0, 20);
    if ($tag !== '') {
        $where .= " AND instr(',' || tags || ',', ?) > 0";
        $params[] = ',' . $tag . ',';
    }
    $total = $db->prepare('SELECT COUNT(*) FROM articles WHERE ' . $where);
    $total->execute($params);
    $count = (int)$total->fetchColumn();
    if (!empty($_GET['cursor'])) {
        $cursor = json_decode(base64_decode((string)$_GET['cursor'], true) ?: '', true);
        if (!is_array($cursor) || !array_is_list($cursor) || count($cursor) !== 2 || !is_string($cursor[0]) || !is_int($cursor[1])) json_response(['error' => '分页游标无效'], 422);
        $where .= ' AND (created_at, id) < (?, ?)';
        array_push($params, $cursor[0], $cursor[1]);
    }
    $stmt = $db->prepare('SELECT * FROM articles WHERE ' . $where . ' ORDER BY created_at DESC, id DESC LIMIT ' . ($limit + 1));
    $stmt->execute($params);
    $articles = $stmt->fetchAll(PDO::FETCH_ASSOC);
    $more = count($articles) > $limit;
    if ($more) array_pop($articles);
    $last = end($articles);
    $commentCounts = comment_counts($db, 'article', array_column($articles, 'id'));
    foreach ($articles as &$article) { $article = article_summary($article); $article['comment_count'] = $commentCounts['article:' . $article['id']] ?? 0; }
    unset($article);
    $result = ['articles' => $articles, 'total' => $count, 'next_cursor' => $more ? base64_encode(json_encode([$last['created_at'], (int)$last['id']])) : null];
    if (!$draft && empty($_GET['cursor'])) {
        $tags = [];
        foreach ($db->query("SELECT tags, COUNT(*) AS n FROM articles WHERE draft = 0 AND tags <> '' GROUP BY tags") as $row) {
            foreach (explode(',', $row['tags']) as $name) $tags[$name] = ($tags[$name] ?? 0) + (int)$row['n'];
        }
        $result['tags'] = $tags;
    }
    json_response($result);
}


/** 访客已点赞的动态 id 列表（token 为浏览器 localStorage 生成的随机串） */
function api_likes(PDO $db, bool $admin): never {
    $token = (string)($_GET['token'] ?? '');
    $liked = [];
    if (preg_match('/^[a-f0-9]{16,64}$/', $token)) {
        $stmt = $db->prepare('SELECT post_id FROM post_likes WHERE token = ?');
        $stmt->execute([$token]);
        $liked = array_map('intval', $stmt->fetchAll(PDO::FETCH_COLUMN));
    }
    json_response(['liked' => $liked]);
}
