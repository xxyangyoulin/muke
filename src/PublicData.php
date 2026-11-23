<?php
declare(strict_types=1);

function public_options(): array {
    $options = [];
    foreach (['locale', 'cover_badge', 'date_format', 'date_format_short', 'site_subtitle', 'site_tagline', 'site_description', 'feed_time_style', 'feed_time_custom', 'feed_hide_time', 'comment_moderation', 'comment_scope'] as $key) $options[$key] = (string)opt($key);
    foreach (['post_max_chars', 'post_max_images', 'upload_max_mb', 'article_max_bytes', 'thumb_size', 'thumb_quality', 'avatar_size', 'reading_chars_per_minute'] as $key) $options[$key] = (int)opt($key);
    $options['comment_provider'] = comment_provider();
    $options['comment_embed_url'] = comment_embed_url();
    $options['comment_rate_seconds'] = (int)opt('comment_rate_seconds');
    return $options;
}

function public_plugins(): array {
    return array_map(fn($plugin) => array_filter(['name' => $plugin['name'] ?? $plugin['id'], 'full_images' => !empty($plugin['post_images_full']), 'color' => plugin_badge_color($plugin)]), plugin_list());
}
