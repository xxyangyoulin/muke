<?php
$path = rtrim(parse_url($_SERVER['REQUEST_URI'], PHP_URL_PATH), '/') ?: '/';
// 静态资源直通（cli-server）：发协商缓存头，避免浏览器对 ESM 子模块的启发式缓存吃到旧代码
if (PHP_SAPI === 'cli-server' && preg_match('#^/(assets|media|thumbnails)/[a-zA-Z0-9._/-]+$#', $path) && is_file(__DIR__ . $path)) {
    $ext = strtolower(pathinfo($path, PATHINFO_EXTENSION));
    $mime = ['css' => 'text/css; charset=utf-8', 'js' => 'text/javascript; charset=utf-8', 'json' => 'application/json', 'svg' => 'image/svg+xml', 'png' => 'image/png', 'jpg' => 'image/jpeg', 'jpeg' => 'image/jpeg', 'webp' => 'image/webp', 'avif' => 'image/avif', 'gif' => 'image/gif', 'woff2' => 'font/woff2', 'ico' => 'image/x-icon'][$ext] ?? 'application/octet-stream';
    header('Content-Type: ' . $mime);
    header('Cache-Control: no-cache');
    readfile(__DIR__ . $path);
    exit;
}
require_once dirname(__DIR__) . '/app.php';
if ($path === '/api') api($db);
if ($path === '/image') image_response();
// RSS / sitemap 动态直出（与静态导出共用生成函数），不再依赖导出产物
if ($path === '/rss.xml' || $path === '/sitemap.xml') {
    header('Content-Type: ' . ($path === '/rss.xml' ? 'application/rss+xml' : 'application/xml') . '; charset=utf-8');
    $feedSettings = settings($db);
    $feedProfile = ['name' => $feedSettings['name'] ?? config('site_name'), 'bio' => $feedSettings['bio'] ?? (string)config('profile_bio')];
    if ($path === '/rss.xml') {
        echo rss_xml_build($feedProfile,
            $db->query('SELECT * FROM articles WHERE draft = 0 ORDER BY id DESC')->fetchAll(PDO::FETCH_ASSOC),
            $db->query('SELECT * FROM posts ORDER BY id DESC')->fetchAll(PDO::FETCH_ASSOC));
    } else {
        $feedUrls = ['/' => null, '/articles/' => null];
        foreach (plugin_pages($db) as $feedRoute => $_) $feedUrls[$feedRoute . '/'] = null;
        foreach ($db->query('SELECT id, updated_at, created_at FROM articles WHERE draft = 0 ORDER BY id DESC')->fetchAll(PDO::FETCH_ASSOC) as $feedArticle) {
            $feedUrls['/article/' . $feedArticle['id'] . '/'] = $feedArticle['updated_at'] ?? $feedArticle['created_at'];
        }
        echo sitemap_xml_build($feedUrls);
    }
    exit;
}
if (preg_match('#^/plugin-asset/([a-z0-9_-]+)/(plugin\.(?:js|css))$#', $path, $m)) {
    $asset = dirname(__DIR__) . '/plugins/' . $m[1] . '/' . $m[2];
    if (is_file($asset)) {
        header('Content-Type: ' . (str_ends_with($m[2], '.js') ? 'text/javascript; charset=utf-8' : 'text/css; charset=utf-8'));
        readfile($asset);
        exit;
    }
    http_response_code(404);
    exit;
}
// 同源评论嵌入页（Waline / Twikoo）：独立宽松 CSP，只放行两个公共 CDN
if ($path === '/embed/comments' && in_array(comment_provider(), ['waline', 'twikoo'], true)) {
    header('Content-Type: text/html; charset=utf-8');
    header("Content-Security-Policy: default-src 'self'; script-src 'self' https://unpkg.com https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline' https://unpkg.com https://cdn.jsdelivr.net; img-src * data: blob:; font-src * data:; connect-src *; frame-ancestors 'self'");
    echo comments_embed_html();
    exit;
}
$static = defined('MKBLOG_STATIC');
$articlePage = (bool)preg_match('#^/article/(\d+)/?$#', $path);
$pluginPages = plugin_pages($db); // 已启用插件声明的公开页面（路由放行 / 导航 / 导出共用）
if (!$static && !in_array($path, ['/', '/articles', '/admin'], true) && !isset($pluginPages[rtrim($path, '/')]) && !$articlePage) { http_response_code(404); echo '页面不存在'; exit; }
if (!$static) {
    header('X-Content-Type-Options: nosniff');
    header('X-Frame-Options: DENY');
    header("Content-Security-Policy: default-src 'self'; img-src 'self' data: blob:; style-src 'self'; script-src 'self'; base-uri 'self'; frame-ancestors 'none'; form-action 'self'" . comment_csp_extra());
    if (session_status() === PHP_SESSION_ACTIVE) session_write_close();
}
$profileName = (string)(settings($db)['name'] ?? config('site_name'));
$profileHandle = (string)(settings($db)['handle'] ?? '');
// SEO 元数据：导出时由 export_bake_pages 预设 \$seoDesc/\$seoImg/\$seoType（文章摘要/封面等），动态访问用站点默认值
// canonical/og:url 仅在配置了 site_url 时输出（避免动态站反射请求 Host 头）
$siteUrl = rtrim((string)opt('site_url'), '/');
$seoDesc = isset($seoDesc) ? $seoDesc : h((string)opt('site_description'));
$seoImg = isset($seoImg) ? $seoImg : '';
$seoType = isset($seoType) ? $seoType : 'website';
$seoTitle = isset($seoTitle) ? $seoTitle : config('site_name') . ' · ' . opt('site_tagline');
$canonical = $siteUrl !== '' ? $siteUrl . ($path === '/' ? '/' : $path . '/') : '';
$avatarFile = DATA_DIR . '/avatar.webp';
$avatarHtml = is_file($avatarFile)
    ? '<img class="avatar small" src="/image?avatar=1&v=' . filemtime($avatarFile) . '" alt="' . h($profileName) . '">'
    : '<span class="avatar small">' . h(mb_substr($profileName, 0, 1)) . '</span>';
?>
<!doctype html>
<html lang="<?= h((string)opt('locale')) ?>"<?= $static ? ' data-static="true"' : '' ?>>
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="theme-color" content="#f6f7f9"><meta name="description" content="<?= $seoDesc ?>">
<title><?= h((string)$seoTitle) ?></title>
<?= $canonical ? '<link rel="canonical" href="' . h($canonical) . '">' : '' ?>
<meta property="og:site_name" content="<?= h($profileName) ?>"><meta property="og:type" content="<?= h($seoType) ?>"><meta property="og:title" content="<?= h((string)$seoTitle) ?>"><meta property="og:description" content="<?= $seoDesc ?>"><?= $canonical ? '<meta property="og:url" content="' . h($canonical) . '">' : '' ?><meta property="og:locale" content="zh_CN"><?= $seoImg ? '<meta property="og:image" content="' . h($seoImg) . '">' : '' ?>
<meta name="twitter:card" content="<?= $seoImg ? 'summary_large_image' : 'summary' ?>"><meta name="twitter:title" content="<?= h((string)$seoTitle) ?>"><meta name="twitter:description" content="<?= $seoDesc ?>"><?= $seoImg ? '<meta name="twitter:image" content="' . h($seoImg) . '">' : '' ?>
<?= isset($seoJsonLd) ? '<script type="application/ld+json">' . $seoJsonLd . '</script>' : '' ?>
<link rel="alternate" type="application/rss+xml" title="<?= h($profileName) ?> 的动态" href="<?= h(site_absolute_url('/rss.xml')) ?>">
<link rel="icon" href="/assets/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="/assets/style.css?v=<?= filemtime(__DIR__ . '/assets/style.css') ?>"><?php foreach (plugin_list() as $pid => $p) { if (empty(plugin_config($db, $pid)['_enabled'])) continue; $pdir = dirname(__DIR__) . "/plugins/$pid/"; if (is_file($pdir . "plugin.css")) echo '<link rel="stylesheet" href="/plugin-asset/' . h($pid) . '/plugin.css?v=' . filemtime($pdir . "plugin.css") . '">'; } ?><script src="/assets/registry.js?v=<?= filemtime(__DIR__ . '/assets/registry.js') ?>"></script>
<script type="module" src="/assets/app.js?v=<?= filemtime(__DIR__ . '/assets/app.js') ?>"></script><?php foreach (plugin_list() as $pid => $p) { if (empty(plugin_config($db, $pid)['_enabled'])) continue; $pdir = dirname(__DIR__) . "/plugins/$pid/"; if (is_file($pdir . "plugin.js")) echo '<script type="module" src="/plugin-asset/' . h($pid) . '/plugin.js?v=' . filemtime($pdir . "plugin.js") . '"></script>'; } ?>
</head>
<body>
<div class="app-shell">
<aside class="sidebar">
<a class="brand" href="/" aria-label="<?= h((string)config('site_name')) ?> 首页" title="<?= h((string)config('site_name')) ?>"><span class="brand-symbol">m<span>·</span></span><span class="brand-text"><?= h((string)config('site_name')) ?><small><?= h((string)config('site_subtitle')) ?></small></span></a>
<nav aria-label="主导航">
<a href="/" data-nav="feed" title="日常动态"><span data-icon="home"></span><span class="nav-text">日常动态</span><span class="nav-dot"></span></a>
<a href="/articles" data-nav="articles" title="文章随笔"><span data-icon="edit"></span><span class="nav-text">文章随笔</span><span class="nav-dot"></span></a>
<?php foreach ($pluginPages as $route => $info): $nav = $info['page']['nav'] ?? null; if (!$nav) continue; ?>
<a href="<?= h($route) ?>" data-nav="<?= h($info['plugin']) ?>" title="<?= h($nav['label'] ?? $info['page']['title']) ?>"><span data-icon="<?= h($nav['icon'] ?? 'grid') ?>"></span><span class="nav-text"><?= h($nav['label'] ?? $info['page']['title']) ?></span><?= !empty($nav['count']) ? '<span class="nav-count" data-plugin-count="' . h($info['plugin']) . '">—</span>' : '' ?></a>
<?php endforeach; ?>
<?php if (!$static): ?><a href="/admin" data-nav="admin" title="管理后台"><span data-icon="grid"></span><span class="nav-text">管理后台</span></a><?php endif; ?>
</nav>
<?php if ($static): $firstPluginPage = $pluginPages ? array_key_first($pluginPages) : '/articles'; ?>
<a class="primary publish-button" id="new-post" href="<?= h($firstPluginPage) ?>" title="探索风景"><span data-icon="image"></span><span class="nav-text">探索风景</span></a>
<?php else: ?>
<button class="primary publish-button" id="new-post" title="记录此刻"><span data-icon="plus"></span><span class="nav-text">记录此刻</span></button>
<?php endif; ?>
<div class="sidebar-bottom"><?= $avatarHtml ?><div class="sidebar-id"><span class="sidebar-name"><strong data-profile="name"><?= h($profileName) ?></strong><svg viewBox="0 0 22 22" fill="currentColor" aria-label="认证账户" role="img" class="verified-badge"><path d="M20.396 11c-.018-.646-.215-1.275-.57-1.816-.354-.54-.852-.972-1.438-1.246.223-.607.27-1.264.14-1.897-.131-.634-.437-1.218-.882-1.687-.47-.445-1.053-.75-1.687-.882-.633-.13-1.29-.083-1.897.14-.273-.587-.704-1.086-1.245-1.44S11.647 1.62 11 1.604c-.646.017-1.273.213-1.813.568s-.969.854-1.24 1.44c-.608-.223-1.267-.272-1.902-.14-.635.13-1.22.436-1.69.882-.445.47-.749 1.055-.878 1.688-.13.633-.08 1.29.144 1.896-.587.274-1.087.705-1.443 1.245-.356.54-.555 1.17-.574 1.817.02.647.218 1.276.574 1.817.356.54.856.972 1.443 1.245-.224.606-.274 1.263-.144 1.896.13.634.433 1.218.877 1.688.47.443 1.054.747 1.687.878.633.132 1.29.084 1.897-.136.274.586.705 1.084 1.246 1.439.54.354 1.17.551 1.816.569.647-.016 1.276-.213 1.817-.567s.972-.854 1.245-1.44c.604.239 1.266.296 1.903.164.636-.132 1.22-.447 1.68-.907.46-.46.776-1.044.908-1.681s.075-1.299-.165-1.903c.586-.274 1.084-.705 1.439-1.246.354-.54.551-1.17.569-1.816zM9.662 14.85l-3.429-3.428 1.293-1.302 2.072 2.072 4.4-4.794 1.347 1.246z"/></svg></span><small class="sidebar-handle" data-profile="handle">@<?= h($profileHandle) ?></small></div><?php if (!$static): ?><a href="/admin" aria-label="管理账户" title="管理账户"><span data-icon="more"></span></a><?php endif; ?></div>
</aside>
<main id="main" tabindex="-1"><div class="loading">正在载入…</div></main>
</div>
<div id="toast" role="status" aria-live="polite"></div>
<div id="lightbox" class="lightbox" role="dialog" aria-modal="true" aria-label="图片预览" hidden>
<div class="lightbox-backdrop" data-lb-close></div>
<div class="lightbox-stage"><div class="lb-loading"><span></span>正在展开风景…</div></div>
<button class="lb-arrow lb-prev" data-lb-prev aria-label="上一张" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m15 4-8 8 8 8"/></svg></button>
<button class="lb-arrow lb-next" data-lb-next aria-label="下一张" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m9 4 8 8-8 8"/></svg></button>
<div class="lightbox-topbar"><span class="lb-counter" id="lb-counter"></span><div class="lb-actions"><button class="lb-button" id="lb-zoom" aria-label="放大查看" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m16.5 16.5 5 5M8 11h6M11 8v6"/></svg></button><a class="lb-button" id="lb-download" download aria-label="下载原图" rel="noopener"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 3v12m-5-5 5 5 5-5M4 16v5h16v-5"/></svg></a><button class="lb-button" data-lb-close aria-label="关闭预览" type="button"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="m6 6 12 12M6 18 18 6"/></svg></button></div></div>
<div class="lb-caption" id="lb-caption"></div>
</div>
<dialog id="edit-dialog"><form id="edit-form"><div class="section-heading"><h2>编辑动态</h2><button class="icon-button" type="button" data-close="edit-dialog" aria-label="关闭"><span data-icon="close"></span></button></div><textarea name="content" maxlength="<?= (int)opt('post_max_chars') ?>" required aria-label="动态内容"></textarea><div id="edit-images" class="compose-images"></div><input type="hidden" name="id"><button class="primary" type="submit">保存修改</button></form></dialog>
<dialog id="plugin-settings-dialog"><form id="plugin-settings-form"><div class="section-heading"><h2 id="plugin-settings-title">插件设置</h2><button class="icon-button" type="button" data-close="plugin-settings-dialog" aria-label="关闭"><span data-icon="close"></span></button></div><p class="small-label" id="plugin-settings-desc"></p><div class="options-grid" id="plugin-settings-fields"></div><div class="dialog-actions"><button class="secondary" type="button" data-close="plugin-settings-dialog">取消</button><button class="primary" type="submit">保存设置</button></div></form></dialog>
<dialog id="delete-dialog"><h2>删除这条动态？</h2><p>删除后无法恢复。</p><div class="dialog-actions"><button class="secondary" data-close="delete-dialog">取消</button><button class="danger" id="confirm-delete">确认删除</button></div></dialog>
<dialog id="plugin-remove-dialog"><h2 id="plugin-remove-title">移除插件？</h2><p>插件将被停用并清除配置；已同步的动态可选择保留或一并删除。</p><label class="switch-line"><input type="checkbox" id="plugin-remove-purge"> 同时删除它已同步的动态（不可恢复）</label><div class="dialog-actions"><button class="secondary" data-close="plugin-remove-dialog">取消</button><button class="danger" id="confirm-plugin-remove">确认移除</button></div></dialog>
</body></html>
