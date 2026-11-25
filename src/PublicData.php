<?php
declare(strict_types=1);

function public_options(): array {
    $options = [];
    foreach (['locale', 'cover_badge', 'date_format', 'date_format_short', 'site_subtitle', 'site_tagline', 'site_description', 'feed_time_style', 'feed_time_custom', 'feed_hide_time', 'comment_moderation', 'comment_scope', 'giscus_repo', 'giscus_repo_id', 'giscus_category_id'] as $key) $options[$key] = (string)opt($key);
    foreach (['post_max_chars', 'post_max_images', 'upload_max_mb', 'article_max_bytes', 'thumb_size', 'thumb_quality', 'avatar_size', 'reading_chars_per_minute'] as $key) $options[$key] = (int)opt($key);
    $options['comment_provider'] = comment_provider();
    $options['comment_embed_url'] = comment_embed_url();
    $options['comment_rate_seconds'] = (int)opt('comment_rate_seconds');
    return $options;
}

/** 公开插件清单：来源徽章渲染所需；传入 $db 时附带「已启用」插件的页面声明（首页 tab / 前端路由用） */
function public_plugins(?PDO $db = null): array {
    $plugins = [];
    foreach (plugin_list() as $id => $plugin) {
        $entry = array_filter(['name' => $plugin['name'] ?? $plugin['id'], 'full_images' => !empty($plugin['post_images_full']), 'color' => plugin_badge_color($plugin)]);
        if ($db !== null && !empty($plugin['page']) && is_array($plugin['page']) && !empty(plugin_config($db, $id)['_enabled'])) {
            $entry['page'] = array_filter(['route' => $plugin['page']['route'] ?? null, 'title' => $plugin['page']['title'] ?? $id, 'tab' => !empty($plugin['page']['tab'])]);
        }
        $plugins[$id] = $entry;
    }
    return $plugins;
}
