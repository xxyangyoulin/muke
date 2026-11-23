<?php
declare(strict_types=1);

// 豆瓣动态插件：把「看过 / 想看 / 在看」列表同步为日常动态。
// 列表页：https://movie.douban.com/people/{用户ID}/collect
return [
    'id' => 'douban',
    'name' => '豆瓣动态',
    'description' => '同步豆瓣的看过 / 想看 / 在看列表为日常动态，含海报封面与条目链接。',
    'label_color' => '#00b51d', // 来源徽章主题色（豆瓣绿）
    'fields' => [
        ['key' => 'user_id', 'label' => '豆瓣用户 ID', 'type' => 'text', 'required' => true, 'pattern' => '/^[a-zA-Z0-9]{1,40}$/', 'hint' => '个人主页地址里的那串数字，如 1234567'],
        ['key' => 'category', 'label' => '同步列表', 'type' => 'select', 'options' => ['collect' => '看过', 'wish' => '想看', 'do' => '在看'], 'default' => 'collect'],
        ['key' => 'post_template', 'label' => '动态文案模板', 'type' => 'text', 'max' => 200, 'default' => '看过《{title}》{rating}', 'hint' => '可用 {title} {year} {rating} {date} {url}'],
        ['key' => 'sync_minutes', 'label' => '同步间隔（分钟）', 'type' => 'number', 'min' => 5, 'max' => 1440, 'default' => 60],
        ['key' => 'history_limit', 'label' => '历史同步上限（条）', 'type' => 'number', 'min' => 1, 'max' => 500, 'default' => 20, 'hint' => '「同步全部历史」一次最多导入这么多条'],
    ],
    'supports_history' => true,
    'card' => 'movie',
    // 静态导出（无 JS 回退）的动态卡片渲染，与 plugin.js 的前端渲染成对
    'render_post' => function (array $post): string {
        $m = post_meta($post);
        $cover = $post['images'][0] ?? null;
        $esc = fn($x) => htmlspecialchars((string)$x, ENT_QUOTES, 'UTF-8');
        return '<a class="post-movie-card" href="' . $esc($post['link'] ?? '#') . '" target="_blank" rel="noopener">'
            . ($cover ? '<img src="' . $esc($cover['thumb'] ?? $cover['url'] ?? '') . '" alt="' . $esc($m['title'] ?? '') . '" loading="lazy">' : '')
            . '<div class="post-movie-info"><strong>' . $esc($m['title'] ?? '') . '</strong>'
            . (($m['intro'] ?? '') !== '' ? '<span class="movie-intro">' . $esc($m['intro']) . '</span>' : '')
            . '<div class="movie-meta">' . (($m['rating'] ?? '') !== '' ? '<span class="movie-stars">' . $esc($m['rating']) . '</span>' : '')
            . (($m['date'] ?? '') !== '' ? '<span class="movie-date">' . $esc($m['date']) . ' ' . $esc($m['category'] ?? '') . '</span>' : '') . '</div>'
            . (($m['comment'] ?? '') !== '' ? '<span class="movie-comment">“' . $esc($m['comment']) . '”</span>' : '')
            . '</div></a>';
    },
    'sync' => function (PDO $db, array $config, array $options = []): array {
        $uid = trim((string)($config['user_id'] ?? ''));
        if ($uid === '') return [];
        $category = in_array($config['category'] ?? '', ['collect', 'wish', 'do'], true) ? $config['category'] : 'collect';
        $label = ['collect' => '看过', 'wish' => '想看', 'do' => '在看'][$category];
        $template = (string)($config['post_template'] ?? '看过《{title}》{rating}');
        $all = !empty($options['all']);
        $limit = $all ? max(1, min(500, (int)($config['history_limit'] ?? 20))) : 30;
        $items = [];
        for ($page = 0; $page < 20 && count($items) < $limit; $page++) {
            $url = "https://movie.douban.com/people/{$uid}/{$category}?start=" . ($page * 30) . "&count=30&sortby=time";
            $html = http_fetch($url, 20, ['Referer' => 'https://movie.douban.com/'], true); // 管理员配置源，允许内网
            if ($html === '') break;
            $chunks = preg_split('~<div class="item comment-item"~', $html);
            array_shift($chunks);
            if (!$chunks) break;
            foreach ($chunks as $block) {
                if (!preg_match('~https?://movie\.douban\.com/subject/(\d+)/?~', $block, $m)) continue;
                $subjectId = $m[1];
                $subjectUrl = "https://movie.douban.com/subject/{$subjectId}/";
                $title = '';
                if (preg_match('~<li class="title">\s*<a[^>]*>(.+?)</a>~s', $block, $m)) $title = trim(preg_replace('/\s+/u', ' ', strip_tags($m[1])));
                if ($title === '') continue;
                $year = '';
                if (preg_match('~class="intro">\s*(\d{4})-~', $block, $m)) $year = $m[1];
                $rating = '';
                if (preg_match('~rating(\d)-t~', $block, $m)) $rating = str_repeat('★', (int)$m[1]) . str_repeat('☆', 5 - (int)$m[1]);
                $intro = '';
                if (preg_match('~<li class="intro">(.+?)</li>~s', $block, $m)) $intro = trim(preg_replace('/\s+/u', ' ', strip_tags($m[1])));
                $comment = '';
                if (preg_match('~<span class="comment">(.+?)</span>~s', $block, $m)) $comment = trim(preg_replace('/\s+/u', ' ', strip_tags($m[1])));
                $date = '';
                if (preg_match('~<span class="date">(\d{4}-\d{2}-\d{2})</span>~', $block, $m)) $date = $m[1];
                $cover = '';
                if (preg_match('~<div class="pic">.*?<img[^>]+src="([^"]+)"~s', $block, $m)) $cover = $m[1];
                $content = strtr($template, ['{title}' => $title, '{year}' => $year, '{rating}' => trim($rating), '{date}' => $date, '{url}' => $subjectUrl, '{category}' => $label]);
                $images = [];
                $coverRef = fetch_remote_image($cover, 0, ['Referer' => 'https://movie.douban.com/']);
                if ($coverRef !== null) $images[] = $coverRef;
                $items[] = [
                    'key' => "douban:{$category}:{$subjectId}",
                    'content' => trim($content),
                    'created_at' => $date !== '' ? date(DATE_ATOM, strtotime($date . ' 21:00:00')) : date(DATE_ATOM),
                    'link' => $subjectUrl,
                    'images' => $images,
                    'meta' => ['card' => 'movie', 'title' => $title, 'year' => $year, 'rating' => trim($rating), 'date' => $date, 'intro' => $intro, 'comment' => $comment, 'category' => $label],
                ];
                if (count($items) >= $limit) break;
            }
            if (count($items) >= $limit) break;
        }
        return $items;
    },
];
