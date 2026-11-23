<?php
declare(strict_types=1);

// 微博动态插件：把一条微博 RSS/Atom 订阅源（如 RSSHub 的 /weibo/user/:uid 路由）同步为日常动态。
// 卡片为微博样式（自定义前端渲染在 plugin.js，静态导出用下方 render_post）。
return [
    'id' => 'weibo',
    'name' => '微博动态',
    'description' => '通过 RSS/Atom 订阅源（推荐 RSSHub 微博路由）同步微博内容为动态，微博样式卡片。',
    'label_color' => '#ff8200', // 来源徽章主题色（微博橙）
    'fields' => [
        ['key' => 'feed_url', 'label' => '订阅源地址', 'type' => 'text', 'required' => true, 'max' => 500, 'pattern' => '#^https?://#', 'hint' => '如 https://rsshub.app/weibo/user/1234567890'],
        ['key' => 'author', 'label' => '博主昵称', 'type' => 'text', 'max' => 40, 'hint' => '卡片上显示的名字，留空用条目作者'],
        ['key' => 'sync_minutes', 'label' => '同步间隔（分钟）', 'type' => 'number', 'min' => 5, 'max' => 1440, 'default' => 30],
        ['key' => 'max_items', 'label' => '每次最多同步（条）', 'type' => 'number', 'min' => 1, 'max' => 50, 'default' => 20],
        ['key' => 'show_source', 'label' => '显示「原文」跳转', 'type' => 'select', 'options' => ['1' => '开启', '0' => '关闭'], 'default' => '1'],
    ],
    // 配置变更时重烙已有动态的 hide_source 旗标（开关即时生效，无需清空重同步）
    'on_config_change' => function (PDO $db, array $config): void {
        $hide = (($config['show_source'] ?? '1') === '1') ? 0 : 1;
        $rows = $db->query("SELECT id, meta FROM posts WHERE source = 'weibo'")->fetchAll(PDO::FETCH_ASSOC);
        $upd = $db->prepare('UPDATE posts SET meta = ? WHERE id = ?');
        foreach ($rows as $r) {
            $m = json_decode($r['meta'] ?: '[]', true) ?: [];
            if (($m['hide_source'] ?? 0) === $hide) continue;
            $m['hide_source'] = $hide;
            $upd->execute([json_encode($m, JSON_UNESCAPED_UNICODE), $r['id']]);
        }
    },
    'supports_history' => true, // 「同步全部历史」入口（RSS 源本身只有最近条目，all 模式放宽上限）
    'sync' => function (PDO $db, array $config, array $options = []): array {
        $url = trim((string)($config['feed_url'] ?? ''));
        if (!preg_match('#^https?://#', $url)) return [];
        $xml = http_fetch($url, 20, [], true); // feed_url 为管理员显式配置源，允许内网（本地 RSSHub）
        if ($xml === '') return [];
        $doc = @simplexml_load_string($xml, 'SimpleXMLElement', LIBXML_NONET); // 禁外部实体加载（XXE）
        if (!$doc) return [];
        $author = trim((string)($config['author'] ?? ''));
        $limit = max(1, min(50, (int)($config['max_items'] ?? 20)));
        if (!empty($options['all'])) $limit = 50;
        $items = [];
        $entries = isset($doc->channel->item) ? $doc->channel->item : (isset($doc->entry) ? $doc->entry : []);
        foreach ($entries as $entry) {
            $link = trim((string)($entry->link ?? $entry->link['href'] ?? ''));
            if ($link === '') continue;
            // 微博条目链接形如 .../status/1234567890 —— 取状态 ID 做幂等键；退化用链接哈希
            $statusId = preg_match('#/status/(\d+)#', $link, $m) ? $m[1] : substr(sha1($link), 0, 12);
            $title = trim((string)($entry->title ?? ''));
            // 注意 SimpleXML 陷阱：不存在的 namespaced 子节点返回空对象而非 null，?? 不会回退 —— 先转字符串判空
            $ce = (string)$entry->children('content', true)->encoded;
            $html = $ce !== '' ? $ce : (string)($entry->summary ?? $entry->description ?? '');
            $text = trim(preg_replace('/\s+/u', ' ', strip_tags($html)) ?: $title);
            if ($text === '') $text = $title;
            if ($text === '') continue;
            $date = (string)($entry->pubDate ?? $entry->updated ?? '');
            $ts = $date !== '' ? @strtotime($date) : false;
            // 提取正文里的图片并下载入库（Referer 过防盗链；minSize 滤掉 emoji/视频占位等小图标）
            $images = [];
            if (preg_match_all('#<img[^>]+src=["\']([^"\']+)#', $html, $imgs)) {
                foreach (array_slice($imgs[1], 0, 4) as $src) {
                    $ref = fetch_remote_image($src, 120, ['Referer' => 'https://weibo.com/']);
                    if ($ref !== null) $images[] = $ref;
                }
            }
            $items[] = [
                'key' => 'weibo:' . $statusId,
                'content' => mb_substr($text, 0, 500),
                'created_at' => date(DATE_ATOM, $ts !== false ? (int)$ts : time()),
                'link' => $link,
                'images' => $images,
                'meta' => ['card' => 'weibo', 'name' => $author !== '' ? $author : (trim((string)($entry->author->name ?? '')) ?: '微博'), 'text' => mb_substr($text, 0, 300), 'hide_source' => (($config['show_source'] ?? '1') === '1') ? 0 : 1],
            ];
            if (count($items) >= $limit) break;
        }
        return $items;
    },
    // 静态导出（无 JS）时的微博卡片（日期/原文由外层动态框架渲染）
    'render_post' => function (array $post): string {
        $m = post_meta($post);
        $cover = $post['images'][0] ?? null;
        $esc = fn($x) => htmlspecialchars((string)$x, ENT_QUOTES, 'UTF-8');
        return '<div class="post-weibo-card"><p class="weibo-text"><span class="weibo-brand">微博</span>'
            . '<p class="weibo-text">' . $esc($m['text'] ?? $post['content']) . '</p>'
            . ($cover ? '<img class="weibo-image" src="' . $esc($cover['thumb'] ?? $cover['url'] ?? '') . '" alt="" loading="lazy">' : '') . '</div>';
    },
];
