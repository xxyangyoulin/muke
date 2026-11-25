<?php
declare(strict_types=1);

const IMAGE_MIMES = ['image/jpeg', 'image/png', 'image/webp', 'image/gif', 'image/avif'];
const MIME_EXT = ['image/jpeg' => 'jpg', 'image/png' => 'png', 'image/webp' => 'webp', 'image/gif' => 'gif', 'image/avif' => 'avif'];

function thumb_magick(string $input, string $output): string {
    $rssThumb = DATA_DIR . '/rss-thumbs/' . basename($input) . '.webp';
    if (is_file($rssThumb)) return 'cp ' . escapeshellarg($rssThumb) . ' ' . escapeshellarg(str_starts_with($output, 'webp:') ? substr($output, 5) : $output);
    $size = (int)opt('thumb_size');
    $quality = (int)opt('thumb_quality');
    // 不用 [0] 首帧语法：大图会触发 IM 无扩展名临时文件解码失败；单帧格式无需指定，动图默认取首帧
    return 'magick ' . escapeshellarg($input) . ' -auto-orient -thumbnail ' . escapeshellarg("{$size}x{$size}>") . ' -strip -quality ' . escapeshellarg((string)$quality) . ' ' . escapeshellarg($output) . ' 2>/dev/null';
}

function save_uploaded_image(array $files, int $index): string {
    if (($files['error'][$index] ?? UPLOAD_ERR_NO_FILE) !== UPLOAD_ERR_OK) throw new RuntimeException('图片上传失败');
    $maxMb = (int)opt('upload_max_mb');
    if ($files['size'][$index] > $maxMb * 1024 * 1024) throw new RuntimeException("单张图片不能超过 {$maxMb}MB");
    $info = @getimagesize($files['tmp_name'][$index]);
    if (!$info || !in_array($info['mime'], IMAGE_MIMES, true)) throw new RuntimeException('仅支持 JPEG / PNG / WebP / GIF / AVIF 图片');
    if (!is_dir(DATA_DIR . '/uploads')) mkdir(DATA_DIR . '/uploads', 0755, true);
    $name = bin2hex(random_bytes(16)) . '.' . MIME_EXT[$info['mime']];
    if (!move_uploaded_file($files['tmp_name'][$index], DATA_DIR . '/uploads/' . $name)) throw new RuntimeException('图片保存失败');
    return 'up:' . $name;
}

function delete_image_file(string $ref): void {
    if (!preg_match('/^up:([a-f0-9]{32}\.(?:jpg|png|webp|gif|avif))$/', $ref, $match)) return;
    global $db;
    $name = $match[1];
    $used = $db->prepare('SELECT 1 FROM articles WHERE images LIKE ? UNION ALL SELECT 1 FROM article_revisions WHERE images LIKE ? UNION ALL SELECT 1 FROM posts WHERE images LIKE ? LIMIT 1');
    $used->execute(['%"' . $name . '"%', '%"' . $name . '"%', '%"up:' . $name . '"%']);
    if (!$used->fetchColumn()) {
        $path = DATA_DIR . '/uploads/' . $name;
        if (is_file($path)) @unlink(DATA_DIR . '/thumbs/' . hash('sha256', $path . filemtime($path) . filesize($path)) . '.webp');
        @unlink($path);
        @unlink(DATA_DIR . '/rss-thumbs/' . $name . '.webp');
    }
}

function send_image_file(string $path, ?string $downloadName = null): never {
    $size = @getimagesize($path);
    if (!$size || !in_array($size['mime'], IMAGE_MIMES, true)) { http_response_code(404); exit; }
    session_write_close();
    if (isset($_GET['thumb'])) {
        $cache = DATA_DIR . '/thumbs';
        if (!is_dir($cache)) mkdir($cache, 0755, true);
        $target = $cache . '/' . hash('sha256', $path . filemtime($path) . filesize($path)) . '.webp';
        if (!is_file($target)) {
            $temp = tempnam($cache, 'thumb-');
            exec(thumb_magick($path, 'webp:' . $temp), $output, $code);
            if ($code === 0 && filesize($temp) > 0) rename($temp, $target);
            else unlink($temp);
        }
        if (is_file($target)) { $path = $target; $size['mime'] = 'image/webp'; }
    }
    $etag = '"' . hash('sha256', $path . filemtime($path) . filesize($path)) . '"';
    header('ETag: ' . $etag);
    header('Cache-Control: public, max-age=60');
    if (($_SERVER['HTTP_IF_NONE_MATCH'] ?? '') === $etag) { http_response_code(304); exit; }
    header('Content-Type: ' . $size['mime']);
    header('X-Content-Type-Options: nosniff');
    if ($downloadName !== null || isset($_GET['download'])) header("Content-Disposition: attachment; filename*=UTF-8''" . rawurlencode($downloadName ?? basename($path)));
    header('Content-Length: ' . filesize($path));
    readfile($path);
    exit;
}

function image_response(): never {
    if (isset($_GET['avatar'])) {
        $avatar = DATA_DIR . '/avatar.webp';
        if (is_file($avatar)) send_image_file($avatar);
        http_response_code(404);
        exit;
    }
    if (isset($_GET['cover'])) {
        $cover = DATA_DIR . '/cover.webp';
        if (is_file($cover)) send_image_file($cover);
        http_response_code(404);
        exit;
    }
    if (isset($_GET['upload'])) {
        $name = $_GET['upload'];
        if (is_string($name) && preg_match('/^[a-f0-9]{32}\.(jpg|png|webp|gif|avif)$/', $name) && is_file(DATA_DIR . '/uploads/' . $name)) send_image_file(DATA_DIR . '/uploads/' . $name);
        http_response_code(404);
        exit;
    }
    // Gravatar 代理：访客邮箱不外泄给第三方；缓存 96px identicon
    if (isset($_GET['gravatar'])) {
        $hash = $_GET['gravatar'];
        if (is_string($hash) && preg_match('/^[a-f0-9]{32}$/', $hash)) {
            $cache = DATA_DIR . '/gravatar/' . $hash;
            if (!is_dir(dirname($cache))) mkdir(dirname($cache), 0755, true);
            if (!is_file($cache)) {
                $data = http_fetch("https://secure.gravatar.com/avatar/{$hash}?s=96&d=identicon&r=g", 10);
                if ($data !== '') file_put_contents($cache, $data);
            }
            if (is_file($cache)) send_image_file($cache);
        }
        http_response_code(404);
        exit;
    }
    // 插件自定义图片服务（如壁纸目录 ?file=<相对路径>）：逐插件询问，越界防护由插件自带
    $relative = $_GET['file'] ?? '';
    if (is_string($relative)) {
        foreach (plugin_list() as $plugin) {
            if (empty($plugin['image_endpoint']) || !is_callable($plugin['image_endpoint'])) continue;
            $path = ($plugin['image_endpoint'])($relative);
            if (is_string($path) && $path !== '') { send_image_file($path, basename($relative)); }
        }
    }
    http_response_code(404);
    exit;
}

function fetch_remote_image(string $url, int $minSize = 0, array $headers = []): ?string {
    if (!preg_match('#^https?://#', $url)) return null;
    // 内容衍生 URL（如 RSS 正文里的图片地址）：默认阻断内网/保留地址（SSRF 防护）
    $data = http_fetch($url, 30, $headers);
    if ($data === '' || strlen($data) > 15 * 1024 * 1024) return null;
    $info = @getimagesizefromstring($data);
    if (!$info || !in_array($info['mime'], IMAGE_MIMES, true)) return null;
    // 过滤表情/图标等占位小图（如微博 42×42 emoji）
    if ($minSize > 0 && ((int)$info[0] < $minSize || (int)$info[1] < $minSize)) return null;
    if (!is_dir(DATA_DIR . '/uploads')) mkdir(DATA_DIR . '/uploads', 0755, true);
    $name = bin2hex(random_bytes(16)) . '.' . MIME_EXT[$info['mime']];
    file_put_contents(DATA_DIR . '/uploads/' . $name, $data);
    return 'up:' . $name;
}
