<?php
declare(strict_types=1);

function config(string $key = ''): mixed {
    static $values = null;
    if ($values === null) {
        $values = require dirname(__DIR__) . '/config.default.php';
        foreach (plugin_list() as $plugin) $values += $plugin['defaults'] ?? [];
        if (is_file(dirname(__DIR__) . '/config.php')) $values = array_merge($values, require dirname(__DIR__) . '/config.php');
        $env = ['MKBLOG_SITE_NAME' => 'site_name', 'MKBLOG_SITE_URL' => 'site_url', 'MKBLOG_SITE_SUBTITLE' => 'site_subtitle', 'MKBLOG_DATA_DIR' => 'data_dir', 'MKBLOG_TIMEZONE' => 'timezone', 'MKBLOG_HTTP_PROXY' => 'http_proxy', 'MKBLOG_TAGLINE' => 'site_tagline', 'MKBLOG_DESC' => 'site_description', 'MKBLOG_LOCALE' => 'locale', 'MKBLOG_POST_MAX_CHARS' => 'post_max_chars', 'MKBLOG_POST_MAX_IMAGES' => 'post_max_images', 'MKBLOG_UPLOAD_MAX_MB' => 'upload_max_mb', 'MKBLOG_ARTICLE_MAX_BYTES' => 'article_max_bytes', 'MKBLOG_THUMB_SIZE' => 'thumb_size', 'MKBLOG_THUMB_QUALITY' => 'thumb_quality', 'MKBLOG_AVATAR_SIZE' => 'avatar_size', 'MKBLOG_SETUP_HOSTS' => 'setup_hosts'];
        foreach (plugin_list() as $plugin) $env += $plugin['env'] ?? [];
        foreach ($env as $var => $name) {
            $override = getenv($var);
            if ($override !== false && $override !== '') $values[$name] = $override;
        }
        if (!str_starts_with((string)$values['data_dir'], '/')) $values['data_dir'] = dirname(__DIR__) . '/' . $values['data_dir'];
        if (is_string($values['setup_hosts'] ?? null)) $values['setup_hosts'] = array_map('trim', explode(',', $values['setup_hosts']));
    }
    return $key === '' ? $values : ($values[$key] ?? '');
}
