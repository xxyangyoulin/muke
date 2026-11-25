<?php
declare(strict_types=1);

// RSS 订阅插件：订阅多个 RSS/Atom 源，条目以订阅卡片展示在独立的「RSS 订阅」标签页；
// 可选并入首页日常动态（默认关闭，timeline_field 声明由核心时间线识别）。
// 订阅管理（添加 / 单源同步 / 退订）在 /rss 页的管理面板；同步频率、历史条数等在后台插件设置。
// 用户删除单条订阅动态由核心立墓碑，之后该条目永不再同步；退订单源时可选连带删除其全部动态。

// 以实际字节数为准：先降低质量，再缩小尺寸；RSS 动图取首帧。
$rss_compress = function (string $source, string $target, int $limit, int $size): void {
    $quality = 80;
    while (true) {
        exec('magick ' . escapeshellarg($source . '[0]') . ' -auto-orient -thumbnail ' . escapeshellarg("{$size}x{$size}>") . ' -strip -quality ' . $quality . ' ' . escapeshellarg('webp:' . $target) . ' 2>/dev/null', $output, $code);
        clearstatcache(true, $target);
        if ($code !== 0 || !is_file($target) || filesize($target) === 0) throw new RuntimeException('RSS 图片压缩失败');
        if (filesize($target) < $limit) return;
        if ($quality > 40) $quality -= 10;
        elseif ($size > 1) $size = max(1, (int)($size * 0.75));
        else throw new RuntimeException('RSS 图片无法压缩到大小限制内');
    }
};
$rss_image = function (string $url, int $minSize) use ($rss_compress): ?string {
    $ref = fetch_remote_image($url, $minSize);
    if ($ref === null) return null;
    $source = DATA_DIR . '/uploads/' . substr($ref, 3);
    $name = bin2hex(random_bytes(16)) . '.webp';
    $target = DATA_DIR . '/uploads/' . $name;
    $thumb = DATA_DIR . '/rss-thumbs/' . $name . '.webp';
    try {
        if (!is_dir(dirname($thumb))) mkdir(dirname($thumb), 0755, true);
        $rss_compress($source, $target, 100 * 1024, 1600);
        $rss_compress($target, $thumb, 50 * 1024, (int)opt('thumb_size'));
        return 'up:' . $name;
    } catch (Throwable $e) {
        @unlink($target);
        @unlink($thumb);
        throw $e;
    } finally { unlink($source); }
};
$rss_cleanup = function (PDO $db, array $config): void {
    if (empty($config['auto_delete'] ?? '1')) return;
    $cutoff = time() - max(1, (int)($config['retention_days'] ?? 30)) * 86400;
    $stmt = $db->prepare("SELECT id, images FROM posts WHERE source = 'rss' AND unixepoch(created_at) < CAST(? AS INTEGER)");
    $stmt->execute([$cutoff]);
    $rows = $stmt->fetchAll(PDO::FETCH_ASSOC);
    if (!$rows) return;
    $db->beginTransaction();
    try {
        foreach ($rows as $row) {
            $db->prepare("DELETE FROM comments WHERE kind = 'post' AND target_id = ?")->execute([$row['id']]);
            $db->prepare('DELETE FROM post_likes WHERE post_id = ?')->execute([$row['id']]);
            $db->prepare('DELETE FROM posts WHERE id = ?')->execute([$row['id']]);
        }
        $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    foreach ($rows as $row) foreach (json_decode($row['images'], true) ?: [] as $ref) delete_image_file($ref);
    auto_deploy_touch($db);
};

// —— 解析：RSS 2.0 / RSS 1.0(RDF) / Atom 统一为 [源标题, [条目…]] ——
$rss_parse = function (string $xml): array {
    $doc = @simplexml_load_string($xml, 'SimpleXMLElement', LIBXML_NONET); // 禁外部实体加载（XXE）
    if (!$doc) throw new RuntimeException('不是有效的 RSS/Atom XML');
    $feedTitle = '';
    $nodes = null;
    if (isset($doc->channel->item)) {        // RSS 2.0
        $feedTitle = trim((string)$doc->channel->title);
        $nodes = $doc->channel->item;
    } elseif (isset($doc->entry)) {          // Atom
        $feedTitle = trim((string)$doc->title);
        $nodes = $doc->entry;
    } elseif (isset($doc->item)) {           // RSS 1.0 / RDF
        $feedTitle = trim((string)($doc->channel->title ?? ''));
        $nodes = $doc->item;
    }
    $entries = [];
    if ($nodes) foreach ($nodes as $node) {
        // SimpleXML 陷阱：不存在的 namespaced 子节点是空对象而非 null，?? 不会回退——转字符串判空后再取下一个
        $link = trim((string)($node->link ?? ''));
        if ($link === '' && isset($node->link)) foreach ($node->link as $candidate) { // Atom：<link rel="alternate" href=…>
            $rel = trim((string)($candidate['rel'] ?? ''));
            if ($rel === '' || $rel === 'alternate') { $link = trim((string)$candidate['href']); break; }
        }
        $guid = trim((string)($node->guid ?? $node->id ?? ''));
        $title = trim((string)preg_replace('/\s+/u', ' ', strip_tags((string)($node->title ?? ''))) ?? '');
        $html = (string)$node->children('content', true)->encoded; // RSS content:encoded
        if ($html === '') $html = trim((string)($node->content ?? '')); // Atom content
        if ($html === '') $html = trim((string)($node->description ?? $node->summary ?? ''));
        $date = trim((string)($node->pubDate ?? ''));
        if ($date === '') $date = trim((string)($node->published ?? $node->updated ?? ''));
        if ($date === '') $date = trim((string)$node->children('dc', true)->date);
        $ts = $date !== '' ? @strtotime($date) : false;
        if ($link === '' && $guid === '' && $title === '') continue;
        // 封面：enclosure 图片优先，其后正文前几张 <img>（fetch 时滤掉小图标）
        $srcs = [];
        if (isset($node->enclosure)) foreach ($node->enclosure as $enclosure) {
            if (str_starts_with(strtolower(trim((string)($enclosure['type'] ?? ''))), 'image/') && preg_match('#^https?://#i', (string)($enclosure['url'] ?? ''))) { $srcs[] = trim((string)$enclosure['url']); break; }
        }
        if (count($srcs) < 3 && $html !== '' && preg_match_all('#<img[^>]+src=["\']([^"\']+)#i', $html, $imgs)) {
            foreach ($imgs[1] as $src) if (count($srcs) >= 3) break; elseif (preg_match('#^https?://#i', $src)) $srcs[] = html_entity_decode($src, ENT_QUOTES | ENT_HTML5, 'UTF-8');
        }
        // 幂等种子：guid > 链接 > 标题+时间
        $seed = $guid !== '' ? $guid : ($link !== '' ? $link : $title . '|' . (int)($ts !== false ? $ts : 0));
        $entries[] = ['key' => substr(sha1($seed), 0, 16), 'title' => $title, 'link' => $link, 'text' => trim((string)preg_replace('/\s+/u', ' ', strip_tags($html)) ?? ''), 'ts' => $ts === false ? time() : (int)$ts, 'srcs' => $srcs];
    }
    if (!$entries) throw new RuntimeException('订阅源里没有可识别的条目');
    return [$feedTitle, $entries];
};

// —— 拉取订阅源（订阅地址为管理员显式配置，允许内网——本地 RSSHub 等）——
$rss_fetch = function (string $url, int $limit) use ($rss_parse): array {
    $xml = http_fetch($url, 20, [], true);
    if ($xml === '') throw new RuntimeException('订阅源无响应或不可达');
    [$feedTitle, $entries] = $rss_parse($xml);
    return [$feedTitle, array_slice($entries, 0, max(1, $limit))];
};

// —— 站点图标（favicon）：Google s2 服务取 64px PNG（自动转格式），存为 up: 引用；失败返回空串 ——
$rss_fetch_icon = function (string $url) use ($rss_image): string {
    $host = (string)(parse_url($url, PHP_URL_HOST) ?: '');
    if ($host === '') return '';
    try { return $rss_image('https://www.google.com/s2/favicons?sz=64&domain=' . rawurlencode($host), 0) ?? ''; } catch (Throwable) { return ''; }
};

// —— 把来源身份（图标引用 + 名称 + @域名）烙进该源已有动态的 meta（图标获取成功时回填一次）——
$rss_backfill_identity = function (PDO $db, string $feedId, string $icon, string $name, string $host): void {
    $handle = preg_replace('~^www\.~', '', $host);
    $stmt = $db->prepare("SELECT id, meta FROM posts WHERE source = 'rss' AND source_key LIKE ?");
    $stmt->execute(['rss:' . $feedId . ':%']);
    $update = $db->prepare('UPDATE posts SET meta = ? WHERE id = ?');
    foreach ($stmt->fetchAll(PDO::FETCH_ASSOC) as $row) {
        $meta = json_decode($row['meta'] ?: '[]', true) ?: [];
        if (($meta['avatar'] ?? null) === $icon && ($meta['handle'] ?? null) === $handle && ($meta['name'] ?? null) === $name) continue;
        $meta['avatar'] = $icon;
        $meta['handle'] = $handle;
        $meta['name'] = $name;
        $update->execute([json_encode($meta, JSON_UNESCAPED_UNICODE), $row['id']]);
    }
};

// —— 单源收集：拉取 → 组装条目（跳过已入库/墓碑条目，不重复下载图片）→ 回写源状态。
//    只收集不入库：sync() 交核心统一入库（幂等 + 事件 + 部署触发），管理操作自行调用 plugin_sync_items。
//    来源身份：首次拿到 favicon 后烙进新条目并回填旧条目（meta.avatar / name / handle）。
$rss_collect_feed = function (PDO $db, array $feed, array $config, bool $all = false, ?array $prefetched = null) use ($rss_fetch, $rss_fetch_icon, $rss_backfill_identity, $rss_image): array {
    $limit = $all ? max(1, min(500, (int)($config['history_limit'] ?? 30))) : max(1, min(50, (int)($config['max_items'] ?? 10)));
    [$feedTitle, $entries] = $prefetched ?? $rss_fetch($feed['url'], $limit);
    $host = (string)(parse_url($feed['url'], PHP_URL_HOST) ?: '');
    $name = $feed['alias'] !== '' ? $feed['alias'] : ($feedTitle !== '' ? $feedTitle : ($host ?: $feed['url']));
    $icon = (string)($feed['icon'] ?? '');
    if ($icon === '') {
        $icon = $rss_fetch_icon($feed['url']);
        if ($icon !== '') {
            $db->prepare('UPDATE rss_feeds SET icon = ? WHERE id = ?')->execute([$icon, $feed['id']]);
            $rss_backfill_identity($db, $feed['id'], $icon, mb_substr($name, 0, 40), $host);
        }
    }
    $prefix = 'rss:' . $feed['id'] . ':';
    $known = [];
    foreach ($db->query("SELECT source_key FROM posts WHERE source = 'rss' AND source_key LIKE " . $db->quote($prefix . '%'))->fetchAll(PDO::FETCH_COLUMN) as $k) $known[substr($k, strlen($prefix))] = true;
    foreach ($db->query("SELECT key FROM plugin_tombstones WHERE plugin = 'rss' AND key LIKE " . $db->quote($prefix . '%'))->fetchAll(PDO::FETCH_COLUMN) as $k) $known[substr($k, strlen($prefix))] = true;
    $items = [];
    foreach ($entries as $entry) {
        if (isset($known[$entry['key']])) continue;
        if (!empty($config['auto_delete'] ?? '1') && $entry['ts'] < time() - max(1, (int)($config['retention_days'] ?? 30)) * 86400) continue;
        $content = $entry['title'] !== '' ? $entry['title'] : mb_substr($entry['text'], 0, 100);
        if ($content === '') continue;
        $images = [];
        foreach ($entry['srcs'] as $src) {
            // 图片尽力而为：订阅源里难免有失效/内网/非图片地址（内网被核心 SSRF 防护阻断），失败只跳过不拖垮条目
            try { $ref = $rss_image($src, 120); } catch (Throwable) { continue; } // minSize 滤掉 emoji / 视频占位等小图
            if ($ref !== null) $images[] = $ref;
        }
        $meta = ['card' => 'rss', 'feed' => $feed['id'], 'name' => mb_substr($name, 0, 40), 'title' => mb_substr($entry['title'], 0, 120), 'summary' => mb_substr($entry['text'], 0, 200)];
        if ($icon !== '') $meta += ['avatar' => $icon, 'handle' => preg_replace('~^www\.~', '', $host)]; // 来源身份三件套
        $items[] = [
            'key' => $prefix . $entry['key'],
            'content' => mb_substr($content, 0, 500),
            'created_at' => date(DATE_ATOM, $entry['ts']),
            'link' => preg_match('#^https?://#i', $entry['link']) ? $entry['link'] : '',
            'images' => $images,
            'meta' => $meta,
        ];
    }
    $db->prepare('UPDATE rss_feeds SET title = ?, last_sync = ?, last_error = ? WHERE id = ?')->execute([$feedTitle, (string)time(), '', $feed['id']]);
    return $items;
};

return [
    'id' => 'rss',
    'name' => 'RSS 订阅',
    'description' => '订阅多个 RSS/Atom 源：条目以订阅卡片展示在独立标签页，可选并入首页动态列表；退订时可连带删除相关动态。',
    'label_color' => '#f26522', // 来源徽章主题色（订阅橙）
    'schema_version' => 3, // v2 期间存在过「版本已记但列未加」的中间态（闭包幂等，重复执行自愈）
    'timeline_field' => 'in_feed', // 该配置关闭时，本插件动态整体退出首页时间线与站点 RSS（默认关闭）
    'supports_history' => true,
    'fields' => [
        ['key' => 'auto_delete', 'label' => '自动删除过期动态', 'type' => 'select', 'options' => ['1' => '开启（默认）', '0' => '关闭'], 'default' => '1'],
        ['key' => 'retention_days', 'label' => '保留天数', 'type' => 'number', 'min' => 1, 'max' => 36500, 'default' => 30, 'hint' => '按条目发布时间计算，同步时删除超过此天数的动态及图片，过期条目不再导入'],
        ['key' => 'sync_minutes', 'label' => '同步间隔（分钟）', 'type' => 'number', 'min' => 5, 'max' => 1440, 'default' => 30],
        ['key' => 'max_items', 'label' => '每次每源同步（条）', 'type' => 'number', 'min' => 1, 'max' => 50, 'default' => 10, 'hint' => '常规同步每个订阅源最多取回的最近条目数'],
        ['key' => 'history_limit', 'label' => '历史同步上限（条/源）', 'type' => 'number', 'min' => 1, 'max' => 500, 'default' => 30, 'hint' => '「同步历史」时每个源一次最多导入的条数；已删除过的条目受墓碑保护不会恢复'],
        ['key' => 'in_feed', 'label' => '并入首页动态列表', 'type' => 'select', 'options' => ['0' => '关闭（默认）', '1' => '开启'], 'default' => '0', 'hint' => '开启后订阅条目同时出现在首页日常动态；关闭时只在「RSS 订阅」标签页展示'],
    ],

    // 插件自有表：订阅源登记（id 为订阅地址哈希；title 为源内发现的标题，alias 为管理员起的名字，icon 为 favicon 的 up: 引用）
    'tables' => function (PDO $db): void {
        $db->exec("CREATE TABLE IF NOT EXISTS rss_feeds (id TEXT PRIMARY KEY, url TEXT NOT NULL, title TEXT NOT NULL DEFAULT '', alias TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL, last_sync INTEGER NOT NULL DEFAULT 0, last_error TEXT NOT NULL DEFAULT '', icon TEXT NOT NULL DEFAULT '')");
        $columns = $db->query('PRAGMA table_info(rss_feeds)')->fetchAll(PDO::FETCH_COLUMN, 1);
        if (!in_array('icon', $columns, true)) $db->exec("ALTER TABLE rss_feeds ADD COLUMN icon TEXT NOT NULL DEFAULT ''"); // v1 → v2
    },

    // 公开页面：核心放行路由 / 渲染首页标签（page.tab）；导航侧栏不重复占位
    'page' => ['route' => '/rss', 'title' => 'RSS 订阅', 'description' => 'RSS 订阅——聚合站长订阅的站点更新。', 'tab' => true],

    // 全量同步：逐源收集（入库交核心，幂等 + 事件 + 部署触发）；单源失败记录到该源并继续，全部失败才让本轮失败
    'sync' => function (PDO $db, array $config, array $options = []) use ($rss_collect_feed, $rss_cleanup): array {
        $rss_cleanup($db, $config);
        $items = [];
        $errors = [];
        $feeds = $db->query('SELECT * FROM rss_feeds ORDER BY created_at, id')->fetchAll(PDO::FETCH_ASSOC);
        foreach ($feeds as $feed) {
            try { $items = array_merge($items, $rss_collect_feed($db, $feed, $config, !empty($options['all']))); }
            catch (Throwable $e) {
                $db->prepare('UPDATE rss_feeds SET last_error = ? WHERE id = ?')->execute([mb_substr($e->getMessage(), 0, 200), $feed['id']]);
                $errors[] = ($feed['alias'] ?: $feed['title'] ?: $feed['url']) . '：' . $e->getMessage();
            }
        }
        if ($errors && count($errors) === count($feeds)) throw new RuntimeException('全部订阅源同步失败——' . implode('；', $errors));
        return $items;
    },

    // 公开数据端点：订阅源列表（管理员另含地址与状态）+ 订阅动态（与 api_posts 同形，游标分页，可按源筛选）
    'data' => function (PDO $db, bool $admin): array {
        $counts = [];
        foreach ($db->query("SELECT substr(source_key, 5, 12) AS fid, COUNT(*) AS n FROM posts WHERE source = 'rss' GROUP BY fid")->fetchAll(PDO::FETCH_KEY_PAIR) as $fid => $n) $counts[$fid] = (int)$n;
        $feeds = [];
        foreach ($db->query('SELECT * FROM rss_feeds ORDER BY created_at, id')->fetchAll(PDO::FETCH_ASSOC) as $f) {
            $row = ['id' => $f['id'], 'title' => $f['alias'] !== '' ? $f['alias'] : ($f['title'] !== '' ? $f['title'] : $f['url']), 'count' => $counts[$f['id']] ?? 0];
            if ($admin) $row += ['url' => $f['url'], 'last_sync' => (int)$f['last_sync'], 'last_error' => $f['last_error']];
            $feeds[] = $row;
        }
        $limit = max(1, min(50, (int)($_GET['limit'] ?? 30)));
        $feed = is_string($_GET['feed'] ?? null) && preg_match('/^[a-f0-9]{12}$/', $_GET['feed']) ? $_GET['feed'] : '';
        $where = "p.source = 'rss'" . ($feed !== '' ? ' AND p.source_key LIKE ' . $db->quote('rss:' . $feed . ':%') : '');
        $total = $db->query('SELECT COUNT(*) FROM posts p WHERE ' . $where)->fetchColumn();
        $params = [];
        if (!empty($_GET['cursor'])) {
            // 非法游标静默忽略（回到第一页），避免数据端点把异常抛成 500
            $cursor = json_decode(base64_decode((string)$_GET['cursor'], true) ?: '', true);
            if (is_array($cursor) && array_is_list($cursor) && count($cursor) === 3 && in_array($cursor[0], [0, 1], true) && is_string($cursor[1]) && is_int($cursor[2])) {
                $where .= ' AND (p.pinned, p.created_at, p.id) < (?, ?, ?)';
                $params = [$cursor[0], $cursor[1], $cursor[2]];
            }
        }
        $stmt = $db->prepare('SELECT p.*, (SELECT COUNT(*) FROM post_likes l WHERE l.post_id = p.id) AS like_count FROM posts p WHERE ' . $where . ' ORDER BY p.pinned DESC, p.created_at DESC, p.id DESC LIMIT ' . ($limit + 1));
        $stmt->execute($params);
        $posts = $stmt->fetchAll(PDO::FETCH_ASSOC);
        $more = count($posts) > $limit;
        if ($more) array_pop($posts);
        $last = end($posts);
        $commentCounts = comment_counts($db, 'post', array_map(fn($p) => (int)$p['id'], $posts));
        foreach ($posts as &$post) {
            $post['images'] = post_images($db, $post['images'] ?? '[]');
            $post['meta'] = post_meta($post);
            if (($post['meta']['avatar'] ?? null) !== null) $post['meta']['avatar'] = post_avatar($db, $post['meta']); // 来源身份图标 → 描述符
            $post['comment_count'] = $commentCounts['post:' . $post['id']] ?? 0;
        }
        unset($post);
        return ['feeds' => $feeds, 'posts' => $posts, 'total' => (int)$total, 'next_cursor' => $more && $last ? base64_encode(json_encode([(int)$last['pinned'], $last['created_at'], (int)$last['id']])) : null];
    },

    // 管理操作：添加订阅（先验证源再登记并立即导入首轮）/ 单源同步 / 退订（可选连带删动态）
    'admin_op' => function (PDO $db, string $op, array $input, array $config) use ($rss_fetch, $rss_collect_feed, $rss_cleanup) {
        if ($op === 'add') {
            if (empty($config['_enabled'])) throw new InvalidArgumentException('插件未启用');
            $url = trim((string)($input['url'] ?? ''));
            if (!preg_match('#^https?://#i', $url) || mb_strlen($url) > 500) throw new InvalidArgumentException('订阅地址需以 http(s):// 开头且不超过 500 字');
            $alias = mb_substr(trim((string)($input['alias'] ?? '')), 0, 40);
            $id = substr(sha1($url), 0, 12);
            [$feedTitle, $entries] = $rss_fetch($url, max(1, min(50, (int)($config['max_items'] ?? 10)))); // 添加时先验证源可用并发现标题，同批直接导入首轮
            $syncLock = fopen(DATA_DIR . '/plugin.sync.lock', 'c');
            flock($syncLock, LOCK_EX); // 与后台同步互斥：并发写动态没有唯一约束兜底
            $events = [];
            $added = 0;
            try {
                $rss_cleanup($db, $config);
                $db->beginTransaction();
                if ($db->query('SELECT 1 FROM rss_feeds WHERE id = ' . $db->quote($id))->fetchColumn()) throw new InvalidArgumentException('该订阅源已在列表中');
                $db->prepare('INSERT INTO rss_feeds (id, url, title, alias, created_at) VALUES (?, ?, ?, ?, ?)')->execute([$id, $url, $feedTitle, $alias, date(DATE_ATOM)]);
                $items = $rss_collect_feed($db, ['id' => $id, 'url' => $url, 'title' => $feedTitle, 'alias' => $alias], $config, false, [$feedTitle, $entries]);
                $added = plugin_sync_items($db, 'rss', $items, $events);
                $db->commit();
            } catch (Throwable $e) {
                if ($db->inTransaction()) $db->rollBack();
                flock($syncLock, LOCK_UN); fclose($syncLock);
                throw $e;
            }
            flock($syncLock, LOCK_UN); fclose($syncLock);
            foreach ($events as $event) plugin_fire('post.saved', $event);
            if ($added > 0) auto_deploy_touch($db);
            return ['id' => $id, 'title' => $alias !== '' ? $alias : ($feedTitle !== '' ? $feedTitle : $url), 'added' => $added];
        }
        if ($op === 'sync') {
            $feedId = (string)($input['feed_id'] ?? '');
            if (!preg_match('/^[a-f0-9]{12}$/', $feedId)) throw new InvalidArgumentException('订阅源标识无效');
            $stmt = $db->prepare('SELECT * FROM rss_feeds WHERE id = ?');
            $stmt->execute([$feedId]);
            $feed = $stmt->fetch(PDO::FETCH_ASSOC);
            if (!$feed) throw new InvalidArgumentException('订阅源不存在');
            $syncLock = fopen(DATA_DIR . '/plugin.sync.lock', 'c');
            flock($syncLock, LOCK_EX);
            try {
                $rss_cleanup($db, $config);
                $added = plugin_sync_items($db, 'rss', $rss_collect_feed($db, $feed, $config));
            }
            catch (Throwable $e) {
                $db->prepare('UPDATE rss_feeds SET last_error = ? WHERE id = ?')->execute([mb_substr($e->getMessage(), 0, 200), $feedId]);
                flock($syncLock, LOCK_UN); fclose($syncLock);
                throw $e;
            }
            flock($syncLock, LOCK_UN); fclose($syncLock);
            if ($added > 0) auto_deploy_touch($db);
            return ['added' => $added];
        }
        if ($op !== 'remove') throw new InvalidArgumentException('操作无效');
        $feedId = (string)($input['feed_id'] ?? '');
        if (!preg_match('/^[a-f0-9]{12}$/', $feedId)) throw new InvalidArgumentException('订阅源标识无效');
        $purge = !empty($input['purge']);
        $icon = '';
        $syncLock = fopen(DATA_DIR . '/plugin.sync.lock', 'c');
        flock($syncLock, LOCK_EX);
        $images = [];
        $deleted = 0;
        try {
            $db->beginTransaction();
            $stmt = $db->prepare('SELECT title, alias, url, icon FROM rss_feeds WHERE id = ?');
            $stmt->execute([$feedId]);
            $feed = $stmt->fetch(PDO::FETCH_ASSOC);
            if (!$feed) throw new InvalidArgumentException('订阅源不存在');
            if ($purge) {
                $prefix = 'rss:' . $feedId . ':%';
                $icon = (string)$feed['icon']; // 动态与源登记一并清掉后，favicon 文件再无引用
                foreach ($db->query('SELECT images, meta FROM posts WHERE source = ' . $db->quote('rss') . ' AND source_key LIKE ' . $db->quote($prefix))->fetchAll(PDO::FETCH_ASSOC) as $row) {
                    foreach (json_decode($row['images'], true) ?: [] as $ref) if (is_string($ref)) $images[] = $ref;
                    $avatar = json_decode($row['meta'] ?: '[]', true)['avatar'] ?? null; // 来源图标随条目 meta 走，不在 images 列
                    if (is_string($avatar) && $avatar !== '') $images[] = $avatar;
                }
                $deleted = (int)$db->query('SELECT COUNT(*) FROM posts WHERE source = ' . $db->quote('rss') . ' AND source_key LIKE ' . $db->quote($prefix))->fetchColumn();
                $db->exec("DELETE FROM comments WHERE kind = 'post' AND target_id IN (SELECT id FROM posts WHERE source = 'rss' AND source_key LIKE " . $db->quote($prefix) . ')');
                $db->exec('DELETE FROM post_likes WHERE post_id IN (SELECT id FROM posts WHERE source = ' . $db->quote('rss') . ' AND source_key LIKE ' . $db->quote($prefix) . ')');
                $db->exec("DELETE FROM posts WHERE source = 'rss' AND source_key LIKE " . $db->quote($prefix));
                $db->exec("DELETE FROM plugin_tombstones WHERE plugin = 'rss' AND key LIKE " . $db->quote($prefix)); // 清史语义与核心「清空动态」一致：之后重订可重新同步
            }
            $db->prepare('DELETE FROM rss_feeds WHERE id = ?')->execute([$feedId]);
            $db->commit();
        } catch (Throwable $e) {
            if ($db->inTransaction()) $db->rollBack();
            flock($syncLock, LOCK_UN); fclose($syncLock);
            throw $e;
        }
        flock($syncLock, LOCK_UN); fclose($syncLock);
        foreach (array_unique(array_merge($images, $icon !== '' ? [$icon] : [])) as $ref) delete_image_file($ref); // up: 引用随条目下载而来，不与其他动态共享
        if ($deleted > 0) auto_deploy_touch($db);
        return ['removed' => $feed['alias'] !== '' ? $feed['alias'] : ($feed['title'] !== '' ? $feed['title'] : $feed['url']), 'deleted' => $deleted];
    },

    // 后台「清空动态」（purge-plugin-posts）后：全部订阅动态已删，favicon 文件失去引用，一并清理
    'hooks' => ['plugin.posts_purged' => function (array $payload): void {
        if (($payload['plugin'] ?? '') !== 'rss') return;
        global $db; // 事件总线只传 payload；请求上下文的 $db 在全局作用域
        foreach ($db->query("SELECT icon FROM rss_feeds WHERE icon <> ''")->fetchAll(PDO::FETCH_COLUMN) as $icon) delete_image_file($icon);
        $db->exec("UPDATE rss_feeds SET icon = ''");
    }],

    // 静态导出（无 JS 回退）的订阅卡片，与 plugin.js 的前端渲染成对
    'render_post' => function (array $post): string {
        $m = post_meta($post);
        $esc = fn($x) => htmlspecialchars((string)$x, ENT_QUOTES, 'UTF-8');
        $cover = $post['images'][0] ?? null;
        $title = (string)($m['title'] ?? '');
        return '<div class="post-rss-card"><p class="rss-line"><span class="rss-feed-tag">' . $esc($m['name'] ?? '') . '</span>'
            . ($title !== '' && preg_match('#^https?://#i', (string)($post['link'] ?? '')) ? '<a class="rss-title" href="' . $esc($post['link']) . '" target="_blank" rel="noopener">' . $esc($title) . '</a>' : ($title !== '' ? '<strong class="rss-title">' . $esc($title) . '</strong>' : ''))
            . '</p>' . (($m['summary'] ?? '') !== '' ? '<p class="rss-summary">' . $esc($m['summary']) . '</p>' : '')
            . ($cover ? '<img class="rss-cover" src="' . $esc($cover['thumb'] ?? $cover['url'] ?? '') . '" alt="" loading="lazy">' : '') . '</div>';
    },

    // 静态导出：订阅动态可能不在首页时间线里（in_feed 关闭），这里单独打包给 /rss 页 SPA；
    // 停用时其动态回到首页时间线（媒体由核心解析），无独立页面，返回空导出。
    'export' => function (PDO $db, string $dest, array &$files, bool $enabled): array {
        if (!$enabled) return ['items' => [], 'refs' => [], 'html' => ''];
        $escape = fn($s) => htmlspecialchars((string)$s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
        $posts = $db->query("SELECT p.*, (SELECT COUNT(*) FROM post_likes l WHERE l.post_id = p.id) AS like_count FROM posts p WHERE p.source = 'rss' ORDER BY p.pinned DESC, p.created_at DESC, p.id DESC")->fetchAll(PDO::FETCH_ASSOC);
        $commentCounts = comment_counts($db, 'post', array_map(fn($p) => (int)$p['id'], $posts));
        $counts = [];
        foreach ($db->query("SELECT substr(source_key, 5, 12) AS fid, COUNT(*) AS n FROM posts WHERE source = 'rss' GROUP BY fid")->fetchAll(PDO::FETCH_KEY_PAIR) as $fid => $n) $counts[$fid] = (int)$n;
        $feeds = [];
        $plugin = plugin_list()['rss'];
        $items = [];
        $body = '<section class="rss-page"><div class="rss-hero"><h1>RSS 订阅。</h1><p>聚合我订阅的站点更新，点击标题可去原文。</p><div class="rss-facts"><span><strong>' . (int)$db->query('SELECT COUNT(*) FROM rss_feeds')->fetchColumn() . '</strong> 个源</span><span><strong>' . count($posts) . '</strong> 条动态</span></div></div><div class="rss-static-list">';
        foreach ($db->query('SELECT * FROM rss_feeds ORDER BY created_at, id')->fetchAll(PDO::FETCH_ASSOC) as $f) $feeds[] = ['id' => $f['id'], 'title' => $f['alias'] !== '' ? $f['alias'] : ($f['title'] !== '' ? $f['title'] : $f['url']), 'count' => $counts[$f['id']] ?? 0];
        // up: 引用 → 静态描述符（原图复制 + 缩略图），正文图片与来源身份图标共用
        $resolve = function (string $ref) use ($dest, &$files): ?array {
            if (!str_starts_with($ref, 'up:')) return null;
            $name = substr($ref, 3);
            $source = DATA_DIR . '/uploads/' . $name;
            if (!is_file($source) || !($info = getimagesize($source))) return null;
            $original = $dest . '/media/uploads/' . $name;
            $thumbnail = $dest . '/thumbnails/up-' . $name . '.webp';
            if (!is_dir(dirname($original))) mkdir(dirname($original), 0755, true);
            if (!is_dir(dirname($thumbnail))) mkdir(dirname($thumbnail), 0755, true);
            if (!is_file($original) && !copy($source, $original)) throw new RuntimeException('订阅动态图片复制失败');
            if (!is_file($thumbnail)) {
                exec(thumb_magick($source, $thumbnail), $output, $code);
                if ($code !== 0) throw new RuntimeException('订阅动态缩略图生成失败，需要 ImageMagick');
            }
            $files[] = $original;
            $files[] = $thumbnail;
            return ['url' => '/media/uploads/' . $name, 'thumb' => '/thumbnails/up-' . $name . '.webp', 'width' => $info[0], 'height' => $info[1], 'bytes' => filesize($source)];
        };
        foreach ($posts as &$post) {
            $images = [];
            foreach (json_decode($post['images'] ?? '[]', true) ?: [] as $ref) if (is_string($ref) && ($hit = $resolve($ref))) $images[] = ['ref' => $ref] + $hit;
            $post['images'] = $images;
            $post['meta'] = post_meta($post);
            $avatarRef = $post['meta']['avatar'] ?? null; // 来源身份图标（favicon 引用）
            if (is_string($avatarRef) && $avatarRef !== '') $post['meta']['avatar'] = $resolve($avatarRef);
            $post['comment_count'] = $commentCounts['post:' . $post['id']] ?? 0;
            $items[] = $post;
            $favicon = is_array($post['meta']['avatar'] ?? null) ? '<img class="avatar" src="' . $escape($post['meta']['avatar']['thumb'] ?? $post['meta']['avatar']['url']) . '" alt="' . $escape($post['meta']['name'] ?? '') . '">' : '';
            $body .= '<article class="post rss-static-item" id="post-' . (int)$post['id'] . '">' . $favicon . '<div class="post-body">' . ($plugin['render_post'])($post) . '<div class="rss-static-meta"><time>' . $escape(date_short_label($post['created_at'])) . '</time>' . ($post['link'] !== '' ? '<a href="' . $escape($post['link']) . '" target="_blank" rel="noopener">原文</a>' : '') . '</div></div></article>';
        }
        unset($post);
        if (!$posts) $body .= '<div class="empty-state"><h3>还没有订阅内容</h3><p>在管理面板添加第一个 RSS/Atom 订阅源。</p></div>';
        $body .= '</div></section>';
        return ['items' => ['feeds' => $feeds, 'posts' => $items], 'refs' => [], 'html' => $body, 'meta' => ['total' => count($items)]];
    },
];
