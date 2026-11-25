<?php
declare(strict_types=1);

// 静态导出管线。export_site() 只做编排，各阶段独立成函数（共享 $ctx 上下文）：
//   prepare(目录/资源) → profile → plugins → posts → articles → json → bake(HTML) → feeds → cleanup
// 注意：本文件位于 src/，仓库根路径 = dirname(__DIR__)。

/** 导出编排：加锁 → 依序执行各阶段 → 汇总 */
function export_site(PDO $db, ?callable $checkCancelled = null): array {
    set_time_limit(0);
    $lock = fopen(DATA_DIR . '/export.lock', 'c');
    if (!flock($lock, LOCK_EX | LOCK_NB)) throw new RuntimeException('已有导出任务正在运行');
    try {
        $root = dirname(__DIR__);
        $stage = $root . '/.export-' . bin2hex(random_bytes(8));
        $previous = $root . '/.dist-previous-' . bin2hex(random_bytes(8));
        $ctx = ['db' => $db, 'root' => $root, 'dest' => $stage, 'files' => []];
        if ($checkCancelled) $checkCancelled();
        export_prepare($ctx);
        if ($checkCancelled) $checkCancelled();
        export_profile($ctx);
        if ($checkCancelled) $checkCancelled();
        export_plugins($ctx);
        if ($checkCancelled) $checkCancelled();
        export_resolve_posts($ctx);
        if ($checkCancelled) $checkCancelled();
        export_resolve_articles($ctx);
        if ($checkCancelled) $checkCancelled();
        if (comment_provider() === 'giscus') {
            $terms = array_merge(array_map(fn($p) => 'post:' . $p['id'], $ctx['posts']), array_map(fn($a) => 'article:' . $a['id'], $ctx['articles']), array_map(fn($p) => 'post:' . $p['id'], $ctx['pluginItems']['rss']['posts'] ?? []));
            $counts = giscus_export_counts($terms, $checkCancelled);
            foreach ($ctx['posts'] as &$post) $post['giscus_counts'] = $counts['post:' . $post['id']];
            unset($post);
            foreach ($ctx['articles'] as &$article) $article['giscus_counts'] = $counts['article:' . $article['id']];
            unset($article);
            if (isset($ctx['pluginItems']['rss']['posts'])) {
                foreach ($ctx['pluginItems']['rss']['posts'] as &$post) $post['giscus_counts'] = $counts['post:' . $post['id']];
                unset($post);
            }
        }
        export_write_json($ctx);
        if ($checkCancelled) $checkCancelled();
        export_bake_pages($ctx);
        if ($checkCancelled) $checkCancelled();
        export_write_feeds($ctx);
        if ($checkCancelled) $checkCancelled();
        export_cleanup_media($ctx);
        foreach (['index.html', 'assets/content.json', 'assets/articles.json', 'rss.xml', 'sitemap.xml'] as $required) {
            if (!is_file($stage . '/' . $required)) throw new RuntimeException('导出产物不完整：' . $required);
        }
        json_decode(file_get_contents($stage . '/assets/content.json'), true, 512, JSON_THROW_ON_ERROR);
        if ($checkCancelled) $checkCancelled();
        $hadPrevious = is_dir($root . '/dist');
        if ($hadPrevious && !rename($root . '/dist', $previous)) throw new RuntimeException('无法保留上一版导出');
        if (!rename($stage, $root . '/dist')) {
            if ($hadPrevious) rename($previous, $root . '/dist');
            throw new RuntimeException('无法发布导出产物');
        }
        if ($hadPrevious) export_remove_directory($previous);
        return ['ok' => true, 'posts' => count($ctx['posts']), 'plugins' => array_map(fn($e) => count($e['items'] ?? []), $ctx['pluginExports']), 'directory' => 'dist/'];
    } finally {
        if (isset($stage) && is_dir($stage)) export_remove_directory($stage);
        flock($lock, LOCK_UN);
        fclose($lock);
    }
}

function export_remove_directory(string $directory): void {
    foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($directory, FilesystemIterator::SKIP_DOTS), RecursiveIteratorIterator::CHILD_FIRST) as $file) {
        if ($file->isDir() && !$file->isLink()) rmdir($file->getPathname());
        else unlink($file->getPathname());
    }
    rmdir($directory);
}

/** 阶段一：目录、插件资源（按启用态）、公共资源、陈旧资源清理 */
function export_prepare(array &$ctx): void {
    [$db, $root, $dest] = [$ctx['db'], $ctx['root'], $ctx['dest']];
    foreach (['assets', 'media/uploads', 'thumbnails'] as $dir) {
        if (!is_dir($dest . '/' . $dir)) mkdir($dest . '/' . $dir, 0755, true);
    }
    if (is_dir($dest . '/admin')) {
        foreach (glob($dest . '/admin/*') as $old) if (is_file($old)) unlink($old);
        @rmdir($dest . '/admin');
    }
    $enabledPlugins = array_keys(array_filter(plugin_list(), fn($p, $id) => !empty(plugin_config($db, $id)['_enabled']), ARRAY_FILTER_USE_BOTH));
    foreach (glob($root . '/plugins/*/plugin.*') as $pluginAsset) {
        if (!preg_match('#/plugins/([a-z0-9_-]+)/plugin\.(js|css)$#', $pluginAsset, $m)) continue;
        if (!in_array($m[1], $enabledPlugins, true)) continue; // 停用插件的资源不导出
        if (!is_dir($dest . '/plugin-assets/' . $m[1])) mkdir($dest . '/plugin-assets/' . $m[1], 0755, true);
        $target = $dest . '/plugin-assets/' . $m[1] . '/plugin.' . $m[2]; // 完整文件名（$m[2] 只是扩展名）
        if (!export_copy_asset($pluginAsset, $target)) throw new RuntimeException('插件资源复制失败'); // css/js 压缩后写入
        $ctx['files'][] = $target;
    }
    // 清理历史版本误存的旧名文件（无 plugin. 前缀的 css/js）
    foreach (glob($dest . '/plugin-assets/*/*') as $staleAsset) {
        if (!in_array(basename($staleAsset), ['plugin.js', 'plugin.css'], true)) @unlink($staleAsset);
    }
    foreach (glob($dest . '/plugin-assets/*', GLOB_ONLYDIR) as $stalePluginAssets) {
        if (!in_array(basename($stalePluginAssets), $enabledPlugins, true)) { foreach (glob($stalePluginAssets . '/*') as $f) @unlink($f); @rmdir($stalePluginAssets); }
    }
    foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($root . '/public/assets', FilesystemIterator::SKIP_DOTS)) as $asset) {
        $relative = substr($asset->getPathname(), strlen($root . '/public/assets'));
        $target = $dest . '/assets' . $relative;
        if (!is_dir(dirname($target))) mkdir(dirname($target), 0755, true);
        if (!export_copy_asset($asset->getPathname(), $target)) throw new RuntimeException('无法复制静态资源'); // css/js 压缩后写入
    }
    foreach (glob($dest . '/assets/*') as $stale) {
        if (is_file($stale) && !is_file($root . '/public/assets/' . basename($stale))) unlink($stale);
    }
}

/** 阶段二：站点资料与头像 */
function export_profile(array &$ctx): void {
    [$db, $dest] = [$ctx['db'], $ctx['dest']];
    $settings = settings($db);
    $avatarSource = DATA_DIR . '/avatar.webp';
    $avatarStatic = is_file($avatarSource) ? '/media/avatar.webp' : null;
    if ($avatarStatic !== null && !is_file($dest . $avatarStatic) && !copy($avatarSource, $dest . $avatarStatic)) throw new RuntimeException('头像复制失败');
    $coverSource = DATA_DIR . '/cover.webp';
    $coverStatic = is_file($coverSource) ? '/media/cover.webp' : null;
    if ($coverStatic !== null && !is_file($dest . $coverStatic) && !copy($coverSource, $dest . $coverStatic)) throw new RuntimeException('封面复制失败');
    $ctx['avatarSource'] = $avatarSource;
    $ctx['avatarStatic'] = $avatarStatic;
    $ctx['coverSource'] = $coverSource;
    $ctx['coverStatic'] = $coverStatic;
    $ctx['profile'] = ['name' => $settings['name'] ?? config('site_name'), 'bio' => $settings['bio'] ?? (string)config('profile_bio'), 'handle' => $settings['handle'] ?? (string)config('profile_handle'), 'avatar' => $avatarStatic, 'cover' => $coverStatic];
    if ($avatarStatic !== null) $ctx['files'][] = $dest . $avatarStatic;
    if ($coverStatic !== null) $ctx['files'][] = $dest . $coverStatic;
}

/** 阶段三：插件导出（媒体复制 + 引用映射 + 无 JS 页面 + 少量配置元数据） */
function export_plugins(array &$ctx): void {
    $db = $ctx['db']; $dest = $ctx['dest'];
    $files = &$ctx['files']; // 注意：引用必须在 LHS，否则变成值拷贝、产物会被收尾清理误删
    $pluginExports = [];
    $pluginRefs = []; // 'wall:<id>' → 静态描述符，供动态图片解析复用
    $pluginItems = []; // content.json 的 plugins 数据
    $pluginMeta = [];  // content.json 的 plugins_meta（各插件导出的少量配置，如瀑布流批量）
    $sources = $db->query("SELECT DISTINCT source FROM posts WHERE source != ''")->fetchAll(PDO::FETCH_COLUMN);
    foreach (plugin_list() as $pid => $plugin) {
        if (empty($plugin['export']) || !is_callable($plugin['export'])) continue;
        $enabled = (bool)plugin_config($db, $pid)['_enabled'];
        if (!$enabled && !in_array($pid, $sources, true)) continue;
        $export = ($plugin['export'])($db, $dest, $files, $enabled);
        $pluginExports[$pid] = $export;
        $pluginRefs += $export['refs'] ?? [];
        $pluginItems[$pid] = $export['items'] ?? [];
        if (isset($export['meta'])) $pluginMeta[$pid] = $export['meta'];
    }
    unset($files);
    $ctx['pluginExports'] = $pluginExports;
    $ctx['pluginRefs'] = $pluginRefs;
    $ctx['pluginItems'] = $pluginItems;
    $ctx['pluginMeta'] = $pluginMeta;
}

/** 单个图片引用 → 静态描述符（上传图复制 + 缩略图；插件引用复用插件导出的文件）；不可解析返回 null */
function export_resolve_media(array &$ctx, string $ref): ?array {
    $dest = $ctx['dest'];
    [$kind, $value] = str_contains($ref, ':') ? explode(':', $ref, 2) : [null, null];
    if ($kind === 'up' && is_file(DATA_DIR . '/uploads/' . $value) && ($info = getimagesize(DATA_DIR . '/uploads/' . $value))) {
        $original = $dest . '/media/uploads/' . $value;
        $thumbnail = $dest . '/thumbnails/up-' . $value . '.webp';
        if (!is_file($original) && !copy(DATA_DIR . '/uploads/' . $value, $original)) throw new RuntimeException('动态图片复制失败');
        if (!is_file($thumbnail)) {
            exec(thumb_magick(DATA_DIR . '/uploads/' . $value, $thumbnail), $output, $code);
            if ($code !== 0) throw new RuntimeException('动态图片缩略图生成失败，需要 ImageMagick');
        }
        $ctx['files'][] = $original;
        $ctx['files'][] = $thumbnail;
        return ['url' => '/media/uploads/' . $value, 'thumb' => '/thumbnails/up-' . $value . '.webp', 'width' => $info[0], 'height' => $info[1], 'bytes' => filesize(DATA_DIR . '/uploads/' . $value)];
    }
    return isset($ctx['pluginRefs'][$ref]) ? $ctx['pluginRefs'][$ref] : null;
}

/** 阶段四：动态图片引用解析为静态地址（上传图复制 + 缩略图，插件引用复用导出文件）；meta.avatar 同步解析 */
function export_resolve_posts(array &$ctx): void {
    $db = $ctx['db'];
    $posts = $db->query('SELECT * FROM posts p WHERE 1 = 1' . feed_source_filter($db) . ' ORDER BY pinned DESC, created_at DESC, id DESC')->fetchAll(PDO::FETCH_ASSOC);
    foreach ($posts as &$post) {
        $images = [];
        foreach (json_decode($post['images'] ?? '[]', true) ?: [] as $ref) {
            if (!is_string($ref)) continue;
            if ($hit = export_resolve_media($ctx, $ref)) $images[] = ['ref' => $ref] + $hit;
        }
        $post['images'] = $images;
        $post['meta'] = post_meta($post);
        $avatarRef = $post['meta']['avatar'] ?? null; // 来源身份图标（如 RSS 订阅的 favicon 引用）
        if (is_string($avatarRef) && $avatarRef !== '') $post['meta']['avatar'] = export_resolve_media($ctx, $avatarRef);
    }
    unset($post);
    $ctx['posts'] = $posts;
}

/** 阶段五：文章渲染、正文图片静态化、封面、陈旧文章页清理 */
function export_resolve_articles(array &$ctx): void {
    [$db, $dest] = [$ctx['db'], $ctx['dest']];
    $files = &$ctx['files'];
    $articles = $db->query('SELECT * FROM articles WHERE draft = 0 ORDER BY id DESC')->fetchAll(PDO::FETCH_ASSOC);
    foreach ($articles as &$article) {
        $html = render_article($article);
        foreach (article_image_refs($html) as $name) {
            if (!is_file(DATA_DIR . '/uploads/' . $name)) continue;
            $target = $dest . '/media/uploads/' . $name;
            if (!is_file($target) && !copy(DATA_DIR . '/uploads/' . $name, $target)) throw new RuntimeException('文章图片复制失败');
            $files[] = $target;
            $html = str_replace('/image?upload=' . $name, '/media/uploads/' . $name, $html);
        }
        $article['html'] = $html;
        $article['excerpt'] = mb_substr(trim(strip_tags(preg_replace('/\s+/', ' ', $html))), 0, 110);
        $article['cover'] = null;
        if (preg_match('#<img[^>]+src="/media/uploads/([a-f0-9]{32}\.(?:jpg|png|webp|gif|avif))"#', $html, $match)) {
            $thumbnail = $dest . '/thumbnails/up-' . $match[1] . '.webp';
            if (!is_file($thumbnail)) {
                exec(thumb_magick($dest . '/media/uploads/' . $match[1], $thumbnail), $output, $code);
                if ($code !== 0) throw new RuntimeException('文章封面缩略图生成失败，需要 ImageMagick');
            }
            $files[] = $thumbnail;
            $article['cover'] = '/thumbnails/up-' . $match[1] . '.webp';
        }
    }
    unset($article, $files);
    foreach (glob($dest . '/article/*/index.html') as $stalePage) {
        $id = basename(dirname($stalePage));
        if (!in_array($id, array_map(fn($a) => (string)$a['id'], $articles), true)) { unlink($stalePage); @rmdir(dirname($stalePage)); }
    }
    $ctx['articles'] = $articles;
}

/** 阶段六：articles.json / content.json 落盘（SPA 静态导航数据源） */
function export_write_json(array &$ctx): void {
    [$db, $dest] = [$ctx['db'], $ctx['dest']];
    $pluginPageMap = plugin_pages($db); // 启用插件的页面（含路由与标题）
    $data = ['state' => ['admin' => false, 'setup' => false, 'profile' => $ctx['profile'], 'plugins' => public_plugins($db), 'options' => public_options()], 'posts' => $ctx['posts'], 'plugins' => $ctx['pluginItems'], 'plugins_meta' => $ctx['pluginMeta']];
    $articlesData = ['articles' => array_map(fn($a) => ['id' => (int)$a['id'], 'title' => $a['title'], 'html' => $a['html'], 'tags' => $a['tags'], 'created_at' => $a['created_at'], 'updated_at' => $a['updated_at'], 'comments' => comment_enabled() ? comments_of($db, 'article', (int)$a['id']) : [], 'giscus_counts' => $a['giscus_counts'] ?? null], $ctx['articles'])];
    file_put_contents($dest . '/assets/articles.json', json_encode($articlesData, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR));
    file_put_contents($dest . '/assets/content.json', json_encode($data, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR));
    $ctx['articlesData'] = $articlesData;
    $ctx['contentData'] = $data; // 烘焙阶段内联进各页面，省去 SPA 首屏的两次 fetch 往返
    $ctx['pluginPageMap'] = $pluginPageMap;
}

/** 阶段七：HTML 烘焙（首页 / 文章列表 / 文章页 / 插件页），含评论只读与第三方嵌入 */
function export_bake_pages(array &$ctx): void {
    [$db, $root, $dest] = [$ctx['db'], $ctx['root'], $ctx['dest']];
    [$articles, $posts, $profile, $pluginPageMap, $pluginExports] = [$ctx['articles'], $ctx['posts'], $ctx['profile'], $ctx['pluginPageMap'], $ctx['pluginExports']];
    $escape = fn($s) => htmlspecialchars((string)$s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
    $siteTitle = h((string)config('site_name'));
    define('MKBLOG_STATIC', true);
    $pages = ['/' => '/index.html', '/articles' => '/articles/index.html'];
    foreach ($pluginPageMap as $route => $info) $pages[$route] = $route . '/index.html';
    // 已停用/移除插件的旧页面清理
    foreach (glob($dest . '/*/index.html') as $stale) {
        $uri = rtrim(substr($stale, strlen($dest), -11), '/');
        if ($uri !== '' && !isset($pages[$uri]) && $uri !== '/index') { unlink($stale); @rmdir(dirname($stale)); }
    }
    foreach ($articles as $article) $pages['/article/' . $article['id']] = '/article/' . $article['id'] . '/index.html';
    foreach ($pages as $uri => $file) {
        $_SERVER['REQUEST_URI'] = $uri;
        $seoDesc = $escape((string)opt('site_description')); $seoImg = ''; $seoType = 'website'; unset($seoTitle, $seoJsonLd);
        if (str_starts_with($uri, '/article/')) {
            $currentArticle = null;
            foreach ($articles as $candidate) if ('/article/' . $candidate['id'] === $uri) { $currentArticle = $candidate; break; }
            if ($currentArticle) {
                $seoDesc = $escape($currentArticle['excerpt'] ?? mb_substr(strip_tags($currentArticle['html']), 0, 110));
                if (!empty($currentArticle['cover'])) $seoImg = site_absolute_url($currentArticle['cover']);
                $seoType = 'article';
                $seoTitle = $currentArticle['title'] . ' · ' . $profile['name'];
                $jsonLd = array_filter([
                    '@context' => 'https://schema.org',
                    '@type' => 'BlogPosting',
                    'headline' => $currentArticle['title'],
                    'description' => $currentArticle['excerpt'] ?? null,
                    'datePublished' => date('c', strtotime($currentArticle['created_at'])),
                    'dateModified' => date('c', strtotime($currentArticle['updated_at'] ?? $currentArticle['created_at'])),
                    'author' => ['@type' => 'Person', 'name' => $profile['name']],
                    'mainEntityOfPage' => site_absolute_url('/article/' . $currentArticle['id'] . '/'),
                    'image' => !empty($currentArticle['cover']) ? site_absolute_url($currentArticle['cover']) : null,
                ], fn($v) => $v !== null);
                // 转义 < 防止 </script> 提前闭合
                $seoJsonLd = str_replace('<', '\u003c', json_encode($jsonLd, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES));
            }
        } elseif (isset($pluginPageMap[$uri]['page']['description'])) {
            $seoDesc = $escape($pluginPageMap[$uri]['page']['description']);
        } elseif ($uri === '/articles') {
            $seoDesc = $escape('文章随笔——一些想认真写下来的事，比动态更长，比记忆更可靠。');
        }
        ob_start();
        include $root . '/public/index.php';
        $html = ob_get_clean();
        $currentArticle = null;
        foreach ($articles as $candidate) if ('/article/' . $candidate['id'] === $uri) { $currentArticle = $candidate; break; }
        $tagline = (string)opt('site_tagline');
        $pageName = $currentArticle ? $currentArticle['title'] : ($pluginPageMap[$uri]['page']['title'] ?? ($uri === '/articles' ? '文章随笔' : $tagline));
        $html = str_replace('<title>' . $siteTitle . ' · ' . h($tagline) . '</title>', '<title>' . ($uri === '/' ? $escape($profile['name']) . ' · ' . $escape($tagline) : $escape($pageName) . ' · ' . $escape($profile['name'])) . '</title>', $html);
        if (!is_dir(dirname($dest . $file))) mkdir(dirname($dest . $file), 0755, true);
        if (isset($pluginPageMap[$uri])) {
            $body = $pluginExports[$pluginPageMap[$uri]['plugin']]['html'] ?? '';
        } elseif ($uri === '/articles') {
            $body = '<section class="articles-page"><div class="articles-heading"><h1>文章随笔。</h1></div><div class="article-list">';
            foreach ($articles as $article) {
                $minutes = max(1, (int)ceil(mb_strlen(strip_tags($article['html'])) / max(50, (int)opt('reading_chars_per_minute'))));
                $body .= '<a class="article-card" href="/article/' . $article['id'] . '/"><h2>' . $escape($article['title']) . '</h2><p>' . $escape($article['excerpt'] ?? '') . '…</p><div class="article-meta"><span>' . $escape(date_short_label($article['created_at'])) . '</span><span>' . $minutes . ' 分钟阅读</span></div>' . (($article['tags'] ?? '') !== '' ? '<span class="article-tags">' . implode('', array_map(fn($t) => '<a class="tag-chip" href="/articles/?tag=' . rawurlencode($t) . '">#' . $escape($t) . '</a>', explode(',', $article['tags']))) . '</span>' : '') . '</a>';
            }
            if (!$articles) $body .= '<div class="empty-state"><h3>第一篇文章，等你落笔</h3><p>这里即将装满更长的故事。</p></div>';
            $body .= '</div></section>';
        } elseif (str_starts_with($uri, '/article/')) {
            $minutes = max(1, (int)ceil(mb_strlen(strip_tags($currentArticle['html'])) / max(50, (int)opt('reading_chars_per_minute'))));
            $body = '<article class="article-page"><div class="article-head"><h1>' . $escape($currentArticle['title']) . '</h1><div class="article-meta"><span>' . $escape(date((string)opt('date_format'), strtotime($currentArticle['created_at']))) . '</span><span>' . $minutes . ' 分钟阅读</span></div>' . (($currentArticle['tags'] ?? '') !== '' ? '<span class="article-tags">' . implode('', array_map(fn($t) => '<a class="tag-chip" href="/articles/?tag=' . rawurlencode($t) . '">#' . $escape($t) . '</a>', explode(',', $currentArticle['tags']))) . '</span>' : '') . '</div><div class="article-content">' . $currentArticle['html'] . '</div><footer class="article-footer"><a class="secondary" href="/articles/">回到文章列表</a></footer></article>';
            $body .= comments_readonly_html($db, 'article', (int)$currentArticle['id']);
            $body .= comments_iframe_html('article', (int)$currentArticle['id']);
        } else {
            $body = '<section class="admin-page"><h1>' . $escape($profile['name']) . '</h1><p>' . $escape($profile['bio']) . '</p><div class="panel">';
            $feed = [];
            foreach ($posts as $post) $feed[] = ['time' => strtotime($post['created_at']), 'kind' => 'post', 'data' => $post];
            foreach ($articles as $article) $feed[] = ['time' => strtotime($article['created_at']), 'kind' => 'article', 'data' => $article];
            usort($feed, fn($a, $b) => ((int)($b['data']['pinned'] ?? 0) <=> (int)($a['data']['pinned'] ?? 0)) ?: ($b['time'] <=> $a['time']) ?: ($b['data']['id'] <=> $a['data']['id']));
            foreach ($feed as $item) {
                if ($item['kind'] === 'article') {
                    $article = $item['data'];
                    $body .= '<article class="post" id="article-' . $article['id'] . '"><div><strong>' . $escape($profile['name']) . '</strong><a class="article-teaser" href="/article/' . $article['id'] . '/"><div class="article-teaser-text"><span class="article-tag">文章</span><h3>' . $escape($article['title']) . '</h3><p>' . $escape($article['excerpt']) . '</p>' . (isset($article['giscus_counts']) ? '<span>' . $article['giscus_counts']['comments'] . ' 评论 · ' . $article['giscus_counts']['likes'] . ' 点赞</span>' : '') . '</div>' . (!empty($article['cover']) ? '<img class="article-teaser-cover" src="' . $escape($article['cover']) . '" alt="" loading="lazy">' : '') . '</a><time>' . $escape(date_short_label($article['created_at'])) . '</time></div></article>';
                    continue;
                }
                $post = $item['data'];
                $images = '';
                $pluginDef = plugin_list()[$post['source'] ?? ''] ?? null;
                $singleCompact = count($post['images']) === 1 && !($pluginDef['post_images_full'] ?? false);
                // 声明了来源身份的插件动态（如 RSS 订阅）显示订阅站点名而非站长名
                $identityName = is_array($post['meta']['avatar'] ?? null) ? (string)($post['meta']['name'] ?? '') : $profile['name'];
                foreach ($post['images'] as $image) { $ar = ($image['width'] ?? 0) > 0 ? max($image['width'] / max(1, $image['height']), 0.5) : 1.5; $images .= '<div class="post-image"' . ($singleCompact ? ' style="aspect-ratio:' . number_format($ar, 3, '.', '') . '"' : '') . '><img src="' . $escape($image['thumb']) . '" alt="' . $escape($image['name'] ?? '动态图片') . '" loading="lazy"></div>'; }
                if ($pluginDef && isset($pluginDef['render_post'])) {
                    $postBody = ($pluginDef['render_post'])($post);
                } else {
                    $postBody = '<p class="post-content">' . $escape($post['content']) . '</p>' . ($images ? '<div class="post-images count' . count($post['images']) . '">' . $images . '</div>' : '');
                }
                $body .= '<article class="post" id="post-' . $post['id'] . '" data-source="' . $escape($post['source'] ?? '') . '"><div><strong>' . $escape($identityName) . '</strong>' . (!empty($post['pinned']) ? '<span class="pinned-label">置顶</span>' : '') . $postBody . (isset($post['giscus_counts']) ? '<p>' . $post['giscus_counts']['comments'] . ' 评论 · ' . $post['giscus_counts']['likes'] . ' 点赞</p>' : '') . '<time>' . $escape(date_short_label($post['created_at'])) . '</time></div></article>';
            }
            if (!$feed) $body .= '<div class="empty-state"><h3>故事，从这一刻开始</h3><p>记录日常的微小瞬间，收集值得停留的风景。</p></div>';
            $body .= '</div></section>';
        }
        if ($ctx['avatarStatic'] !== null) $html = str_replace('/image?avatar=1&v=' . filemtime($ctx['avatarSource']), $ctx['avatarStatic'], $html);
        if (($ctx['coverStatic'] ?? null) !== null) $html = str_replace('/image?cover=1&v=' . filemtime($ctx['coverSource']), $ctx['coverStatic'], $html);
        $html = str_replace('/plugin-asset/', '/plugin-assets/', $html);
        // 烘焙兜底包进 #baked-body：有 JS（registry.js 首帧前打 has-js）时由 CSS 藏起、显示加载占位，SPA 用下方内联 JSON 渲染一次到位，
        // 消除「烘焙结构 → fetch 完再重渲染」的两套结构闪变；无 JS 时兜底原样可见（SEO 不受影响）。
        // JSON 内联为 <script type="application/json"> 数据块（CSP script-src 'self' 不拦非执行块），< 转义防止 </script> 提前闭合
        $jsonTag = fn(array $payload, string $id): string => '<script type="application/json" id="' . $id . '">'
            . str_replace('<', '<', json_encode($payload, JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR))
            . '</script>';
        $loading = '<div class="loading">正在载入…</div>';
        $html = str_replace($loading, $loading
            . $jsonTag($ctx['contentData'], 'embedded-content')
            . $jsonTag($ctx['articlesData'], 'embedded-articles')
            . '<div id="baked-body">' . $body . '</div>', $html);
        if (file_put_contents($dest . $file, minify_html($html)) === false) throw new RuntimeException('HTML 写入失败');
    }
}

/** 阶段八：RSS / sitemap / 评论嵌入页 / 404 / _headers */

/** RSS 聚合（导出与动态路由共用）：文章 + 动态，最新 30 条 */
function rss_xml_build(array $profile, array $articles, array $posts): string {
    $escape = fn($s) => htmlspecialchars((string)$s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
    $feedItems = [];
    foreach ($articles as $article) $feedItems[] = ['title' => $article['title'], 'link' => site_absolute_url('/article/' . $article['id'] . '/'), 'date' => $article['created_at'], 'desc' => $article['excerpt'] ?? ''];
    foreach ($posts as $post) $feedItems[] = ['title' => mb_substr(strip_tags($post['content']), 0, 40), 'link' => site_absolute_url('/#post-' . $post['id']), 'date' => $post['created_at'], 'desc' => strip_tags($post['content'])];
    usort($feedItems, fn($a, $b) => strtotime($b['date']) <=> strtotime($a['date']));
    $rss = '<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>' . $escape($profile['name']) . '</title><link>' . site_absolute_url('/') . '</link><description>' . $escape($profile['bio']) . '</description>';
    foreach (array_slice($feedItems, 0, 30) as $item) {
        $rss .= '<item><title>' . $escape($item['title']) . '</title><link>' . $escape($item['link']) . '</link><pubDate>' . date(DATE_RSS, strtotime($item['date'])) . '</pubDate><description>' . $escape($item['desc']) . '</description></item>';
    }
    return $rss . '</channel></rss>';
}

/** sitemap（导出与动态路由共用）：入参为 路径 => lastmod(null 允许) 映射 */
function sitemap_xml_build(array $urls): string {
    $escape = fn($s) => htmlspecialchars((string)$s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
    $sm = '<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">';
    foreach ($urls as $u => $lastmod) {
        $sm .= '<url><loc>' . $escape(site_absolute_url($u)) . '</loc>';
        if ($lastmod) $sm .= '<lastmod>' . date('Y-m-d', strtotime($lastmod)) . '</lastmod>';
        $sm .= '</url>';
    }
    return $sm . '</urlset>';
}

function export_write_feeds(array &$ctx): void {
    [$dest, $articles, $posts, $profile] = [$ctx['dest'], $ctx['articles'], $ctx['posts'], $ctx['profile']];
    file_put_contents($dest . '/rss.xml', rss_xml_build($profile, $articles, $posts));
    $urls = ['/' => null, '/articles/' => null];
    foreach ($ctx['pluginPageMap'] as $route => $_) $urls[$route . '/'] = null;
    foreach ($articles as $article) $urls['/article/' . $article['id'] . '/'] = $article['updated_at'] ?? $article['created_at'];
    file_put_contents($dest . '/sitemap.xml', sitemap_xml_build($urls));
    file_put_contents($dest . '/robots.txt', "User-agent: *\nAllow: /\n\nSitemap: " . site_absolute_url('/sitemap.xml') . "\n");
    if (in_array(comment_provider(), ['waline', 'twikoo'], true)) {
        if (!is_dir($dest . '/embed/comments')) mkdir($dest . '/embed/comments', 0755, true);
        file_put_contents($dest . '/embed/comments/index.html', comments_embed_html());
    } elseif (is_dir($dest . '/embed')) {
        foreach (glob($dest . '/embed/*') as $staleEmbed) @unlink($staleEmbed);
        @rmdir($dest . '/embed/comments'); @rmdir($dest . '/embed');
    }
    file_put_contents($dest . '/404.html', '<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>页面不存在</title><link rel="stylesheet" href="/assets/style.css"><main class="empty-state"><h1>这片风景还未抵达</h1><p>你访问的页面不存在。</p><a href="/">回到首页</a></main></html>');
    file_put_contents($dest . '/_headers', "/*\n  X-Content-Type-Options: nosniff\n  X-Frame-Options: DENY\n  Referrer-Policy: strict-origin-when-cross-origin\n  Content-Security-Policy: default-src 'self'; img-src 'self' data: blob:; style-src 'self'; script-src 'self'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'" . comment_csp_extra() . "\n" . (in_array(comment_provider(), ['waline', 'twikoo'], true) ? "/embed/*\n  Content-Security-Policy: default-src 'self'; script-src 'self' https://unpkg.com https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline' https://unpkg.com https://cdn.jsdelivr.net; img-src * data: blob:; font-src * data:; connect-src *; frame-ancestors 'self'\n" : ''));
}

/** 阶段九：清理不再被引用的媒体文件 */
function export_cleanup_media(array &$ctx): void {
    $dest = $ctx['dest'];
    foreach ([glob($dest . '/media/*'), glob($dest . '/media/uploads/*'), glob($dest . '/thumbnails/*')] as $group) {
        foreach ($group as $old) if (is_file($old) && !in_array($old, $ctx['files'], true)) unlink($old);
    }
}
