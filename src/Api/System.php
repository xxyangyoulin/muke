<?php
declare(strict_types=1);

/** 登录限速：同 IP 哈希+用户名 15 分钟内失败 5 次锁定（文件计数，无额外表） */
function login_throttled(string $ipHash, string $user): ?int {
    $file = DATA_DIR . '/login.throttle.json';
    $data = is_file($file) ? json_decode((string)file_get_contents($file), true) ?: [] : [];
    $key = substr($ipHash, 0, 16) . '|' . $user;
    $now = time();
    $data = array_filter($data, fn($e) => $now - ($e['t'] ?? 0) < 900);
    $fails = array_values(array_filter($data, fn($e) => ($e['k'] ?? '') === $key && ($e['ok'] ?? 0) === 0));
    if (count($fails) >= 5) {
        $last = max(array_column($fails, 't'));
        return max(1, 900 - ($now - $last));
    }
    return null;
}

function login_record(string $ipHash, string $user, bool $ok): void {
    $file = DATA_DIR . '/login.throttle.json';
    if ($ok) { @unlink($file); return; }
    $data = is_file($file) ? json_decode((string)file_get_contents($file), true) ?: [] : [];
    $data[] = ['k' => substr($ipHash, 0, 16) . '|' . $user, 't' => time(), 'ok' => 0];
    file_put_contents($file, json_encode(array_slice($data, -50)), LOCK_EX);
}

function api_auth(PDO $db, array $input, string $action, bool $admin): never {
    $name = trim((string)($input['username'] ?? ''));
    $password = (string)($input['password'] ?? '');
    if ($action === 'setup') {
        $hosts = (array)config('setup_hosts');
        if (!in_array('*', $hosts, true) && !in_array($_SERVER['REMOTE_ADDR'] ?? '', $hosts, true)) json_response(['error' => '首次建号仅允许来自受信来源（见配置 setup_hosts）'], 403);
        if (mb_strlen($name) < 2 || mb_strlen($name) > 40 || strlen($password) < 10 || strlen($password) > 200) json_response(['error' => '用户名需 2–40 字，密码需 10–200 位'], 422);
        $stmt = $db->prepare('INSERT OR IGNORE INTO users (id, name, password) VALUES (1, ?, ?)');
        $stmt->execute([$name, password_hash($password, PASSWORD_DEFAULT)]);
        if (!$stmt->rowCount()) json_response(['error' => '管理员已设置，请登录'], 409);
        $_SESSION['user'] = 1;
    } else {
        if (time() - ($_SESSION['last_login'] ?? 0) < 2) json_response(['error' => '请稍后再试'], 429);
        $_SESSION['last_login'] = time();
        $th = login_throttled(comment_ip_hash(), $name);
        if ($th !== null) json_response(['error' => '尝试次数过多，请 ' . (int)ceil($th / 60) . ' 分钟后再试'], 429);
        $stmt = $db->prepare('SELECT * FROM users WHERE name = ?');
        $stmt->execute([$name]);
        $user = $stmt->fetch(PDO::FETCH_ASSOC);
        if (!$user || !password_verify($password, $user['password'])) {
            login_record(comment_ip_hash(), $name, false);
            json_response(['error' => '用户名或密码不正确'], 401);
        }
        login_record(comment_ip_hash(), $name, true);
        $_SESSION['user'] = $user['id'];
    }
    session_regenerate_id(true);
    json_response(['ok' => true]);
}

function api_logout(PDO $db, array $input, string $action, bool $admin): never {
    if (session_status() === PHP_SESSION_ACTIVE) {
        $_SESSION = [];
        session_destroy();
        if (ini_get('session.use_cookies')) {
            $p = session_get_cookie_params();
            setcookie(session_name(), '', time() - 42000, $p['path'], $p['domain'], $p['secure'], $p['httponly']);
        }
    }
    json_response(['ok' => true]);
}

function api_profile(PDO $db, array $input, string $action, bool $admin): never {
    foreach (['name' => 40, 'bio' => 160, 'handle' => 30] as $key => $limit) {
        $value = trim((string)($input[$key] ?? ''));
        if ($value === '' || mb_strlen($value) > $limit) json_response(['error' => '请完整填写资料，并遵守字数限制'], 422);
        if ($key === 'handle' && !preg_match('/^[a-zA-Z0-9_]+$/', $value)) json_response(['error' => '用户名仅可包含英文字母、数字和下划线'], 422);
    }
    $stmt = $db->prepare('INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value');
    foreach (['name', 'bio', 'handle'] as $key) $stmt->execute([$key, trim((string)$input[$key])]);
    $stmt->execute(['cover_badge', mb_substr(trim((string)($input['cover_badge'] ?? '')), 0, 30)]); // 封面角标（选填，空=不显示）
    json_response(['ok' => true]);
}

function api_change_password(PDO $db, array $input, string $action, bool $admin): never {
    $current = (string)($input['current'] ?? '');
    $next = (string)($input['next'] ?? '');
    if (strlen($next) < 10 || strlen($next) > 200) json_response(['error' => '新密码需 10–200 位'], 422);
    $stmt = $db->prepare('SELECT password FROM users WHERE id = ?');
    $stmt->execute([$_SESSION['user']]);
    $hash = $stmt->fetchColumn();
    if (!$hash || !password_verify($current, $hash)) json_response(['error' => '当前密码不正确'], 401);
    $db->prepare('UPDATE users SET password = ? WHERE id = ?')->execute([password_hash($next, PASSWORD_DEFAULT), $_SESSION['user']]);
    json_response(['ok' => true]);
}

function api_save_avatar(PDO $db, array $input, string $action, bool $admin): never {
    try {
        $file = $_FILES['avatar'] ?? null;
        if (!$file || ($file['error'] ?? UPLOAD_ERR_NO_FILE) !== UPLOAD_ERR_OK) json_response(['error' => '请选择图片'], 422);
        $maxMb = (int)opt('upload_max_mb');
        if ($file['size'] > $maxMb * 1024 * 1024) json_response(['error' => "图片不能超过 {$maxMb}MB"], 422);
        $info = @getimagesize($file['tmp_name']);
        if (!$info || !in_array($info['mime'], IMAGE_MIMES, true)) json_response(['error' => '仅支持 JPEG / PNG / WebP / GIF / AVIF 图片'], 422);
        $target = DATA_DIR . '/avatar.webp';
        $temp = tempnam(DATA_DIR, 'avatar-');
        $av = (int)opt('avatar_size');
        exec('magick ' . escapeshellarg($file['tmp_name'] . '[0]') . ' -auto-orient -thumbnail ' . escapeshellarg("{$av}x{$av}^") . ' -gravity center -extent ' . escapeshellarg("{$av}x{$av}") . ' -strip -quality 85 ' . escapeshellarg('webp:' . $temp) . ' 2>/dev/null', $output, $code);
        if ($code !== 0 || !filesize($temp)) { if (is_file($temp)) unlink($temp); throw new RuntimeException('头像处理失败，需要 ImageMagick'); }
        rename($temp, $target);
        json_response(['ok' => true, 'avatar' => '/image?avatar=1&v=' . filemtime($target)]);
    } catch (RuntimeException $e) {
        json_response(['error' => $e->getMessage()], 422);
    }
}

function api_remove_avatar(PDO $db, array $input, string $action, bool $admin): never {
    @unlink(DATA_DIR . '/avatar.webp');
    json_response(['ok' => true]);
}

/** 封面图上传：magick 居中裁剪为 1500×500（3:1，X 封面比例），存 data/cover.webp */
function api_save_cover(PDO $db, array $input, string $action, bool $admin): never {
    try {
        $file = $_FILES['cover'] ?? null;
        if (!$file || ($file['error'] ?? UPLOAD_ERR_NO_FILE) !== UPLOAD_ERR_OK) json_response(['error' => '请选择图片'], 422);
        $maxMb = (int)opt('upload_max_mb');
        if ($file['size'] > $maxMb * 1024 * 1024) json_response(['error' => "图片不能超过 {$maxMb}MB"], 422);
        $info = @getimagesize($file['tmp_name']);
        if (!$info || !in_array($info['mime'], IMAGE_MIMES, true)) json_response(['error' => '仅支持 JPEG / PNG / WebP / GIF / AVIF 图片'], 422);
        $target = DATA_DIR . '/cover.webp';
        $temp = tempnam(DATA_DIR, 'cover-');
        exec('magick ' . escapeshellarg($file['tmp_name'] . '[0]') . ' -auto-orient -thumbnail 1500x500^ -gravity center -extent 1500x500 -strip -quality 85 ' . escapeshellarg('webp:' . $temp) . ' 2>/dev/null', $output, $code);
        if ($code !== 0 || !filesize($temp)) { if (is_file($temp)) unlink($temp); throw new RuntimeException('封面处理失败，需要 ImageMagick'); }
        rename($temp, $target);
        json_response(['ok' => true, 'cover' => '/image?cover=1&v=' . filemtime($target)]);
    } catch (RuntimeException $e) {
        json_response(['error' => $e->getMessage()], 422);
    }
}

function api_remove_cover(PDO $db, array $input, string $action, bool $admin): never {
    @unlink(DATA_DIR . '/cover.webp');
    json_response(['ok' => true]);
}

function api_save_settings(PDO $db, array $input, string $action, bool $admin): never {
    $giscus = [];
    foreach (['giscus_repo' => '~^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$~', 'giscus_repo_id' => '/^[A-Za-z0-9_=-]+$/', 'giscus_category_id' => '/^[A-Za-z0-9_=-]+$/'] as $key => $pattern) {
        if (!array_key_exists($key, $input)) continue;
        if (!is_string($input[$key])) json_response(['error' => 'giscus 配置格式无效'], 422);
        $value = trim($input[$key]);
        if ($value !== '' && (strlen($value) > 200 || !preg_match($pattern, $value))) json_response(['error' => 'giscus 仓库或分类 ID 格式无效'], 422);
        $giscus[$key] = $value;
    }
    if (($input['comment_provider'] ?? comment_provider()) === 'giscus') {
        foreach (['giscus_repo', 'giscus_repo_id', 'giscus_category_id'] as $key) {
            if (($giscus[$key] ?? (string)opt($key)) === '') json_response(['error' => '请填写 giscus 仓库、仓库 ID 和分类 ID'], 422);
        }
    }
    foreach ($giscus as $key => $value) save_setting($db, $key, $value);
    $textRules = ['site_subtitle' => 40, 'site_tagline' => 60, 'site_description' => 200, 'date_format' => 40, 'date_format_short' => 40, 'cover_badge' => 30];
    $intRules = ['post_max_chars' => [1, 5000], 'post_max_images' => [1, 9], 'upload_max_mb' => [1, 100], 'article_max_bytes' => [1024, 20971520], 'thumb_size' => [100, 2000], 'thumb_quality' => [1, 100], 'avatar_size' => [100, 1000], 'reading_chars_per_minute' => [50, 3000], 'comment_rate_seconds' => [0, 3600]];
    $saved = 0;
    foreach ($textRules as $key => $limit) {
        if (!array_key_exists($key, $input)) continue;
        $value = trim((string)$input[$key]);
        if (mb_strlen($value) > $limit) json_response(['error' => "{$key} 超出字数限制"], 422);
        save_setting($db, $key, $value);
        $saved++;
    }
    if (array_key_exists('feed_time_style', $input)) {
        $fts = (string)$input['feed_time_style'];
        if (!in_array($fts, ['', 'compact', 'date', 'ago', 'custom'], true)) json_response(['error' => '动态时间格式取值无效'], 422);
        save_setting($db, 'feed_time_style', $fts);
        $saved++;
    }
    if (array_key_exists('feed_hide_time', $input)) {
        save_setting($db, 'feed_hide_time', ((string)$input['feed_hide_time'] === '1') ? '1' : '0');
        $saved++;
    }
    if (array_key_exists('feed_time_custom', $input)) {
        $ftc = trim((string)$input['feed_time_custom']);
        if ($ftc !== '' && (mb_strlen($ftc) > 30 || !preg_match('/^[A-Za-z0-9 \:\-\.\ \/年月日时分秒]*$/', $ftc))) json_response(['error' => '自定义格式含不支持的字符（可用符号见下方说明）'], 422);
        save_setting($db, 'feed_time_custom', $ftc);
        $saved++;
    }
    if (array_key_exists('auto_deploy', $input)) {
        $ad = (string)$input['auto_deploy'];
        if (!in_array($ad, ['0', '1'], true)) json_response(['error' => '自动部署取值无效'], 422);
        save_setting($db, 'auto_deploy', $ad);
        if ($ad === '0') auto_deploy_pending_clear(); // 关闭即取消排队中的自动任务
        $saved++;
    }
    if (array_key_exists('auto_deploy_debounce', $input)) {
        $v = filter_var($input['auto_deploy_debounce'], FILTER_VALIDATE_INT);
        if ($v === false || $v < 1 || $v > 1440) json_response(['error' => '防抖时间需在 1–1440 分钟之间'], 422);
        save_setting($db, 'auto_deploy_debounce', (string)$v);
        $saved++;
    }
    if (array_key_exists('comment_provider', $input)) {
        $provider = (string)$input['comment_provider'];
        if (!in_array($provider, ['', 'off', 'giscus', 'waline', 'twikoo', 'custom'], true)) json_response(['error' => '评论系统取值无效'], 422);
        save_setting($db, 'comment_provider', $provider);
        $saved++;
    }
    if (array_key_exists('comment_moderation', $input)) {
        $m = (string)$input['comment_moderation'];
        if (!in_array($m, ['', 'pre'], true)) json_response(['error' => '审核策略取值无效'], 422);
        save_setting($db, 'comment_moderation', $m);
        $saved++;
    }
    foreach (['smtp_host' => 100, 'smtp_user' => 100, 'smtp_pass' => 200, 'notify_email' => 100, 'akismet_key' => 100] as $mk => $mlimit) {
        if (!array_key_exists($mk, $input)) continue;
        if ($mk === 'smtp_pass' && trim((string)$input[$mk]) === '') continue; // 授权码留空 = 保持已存值（面板不回显密码）
        $mv = trim((string)$input[$mk]);
        if (mb_strlen($mv) > $mlimit) json_response(['error' => "{$mk} 过长"], 422);
        if ($mk === 'notify_email' && $mv !== '' && !filter_var($mv, FILTER_VALIDATE_EMAIL)) json_response(['error' => '通知邮箱格式不正确'], 422);
        save_setting($db, $mk, $mv);
        $saved++;
    }
    if (array_key_exists('smtp_port', $input)) {
        $mp = (int)$input['smtp_port'];
        if ($mp < 1 || $mp > 65535) json_response(['error' => 'SMTP 端口无效'], 422);
        save_setting($db, 'smtp_port', (string)$mp);
        $saved++;
    }
    if (array_key_exists('comment_scope', $input)) {
        $sc = (string)$input['comment_scope'];
        if (!in_array($sc, ['', 'article'], true)) json_response(['error' => '评论范围取值无效'], 422);
        save_setting($db, 'comment_scope', $sc);
        $saved++;
    }
    if (array_key_exists('comment_embed_url', $input)) {
        $embed = trim((string)$input['comment_embed_url']);
        if ($embed !== '' && (mb_strlen($embed) > 200 || !preg_match('#^https?://#', $embed))) json_response(['error' => '评论嵌入地址需以 http(s):// 开头'], 422);
        save_setting($db, 'comment_embed_url', $embed);
        $saved++;
    }
    if (array_key_exists('locale', $input)) {
        $locale = trim((string)$input['locale']);
        if (!preg_match('/^[a-zA-Z]{2,3}(-[a-zA-Z0-9]{2,8})?$/', $locale)) json_response(['error' => 'locale 格式不正确，如 zh-CN'], 422);
        save_setting($db, 'locale', $locale);
        $saved++;
    }
    foreach ($intRules as $key => [$min, $max]) {
        if (!array_key_exists($key, $input)) continue;
        $value = filter_var($input[$key], FILTER_VALIDATE_INT);
        if ($value === false || $value < $min || $value > $max) json_response(['error' => "{$key} 需在 {$min}–{$max} 之间"], 422);
        save_setting($db, $key, (string)$value);
        $saved++;
    }
    json_response(['ok' => true, 'saved' => $saved]);
}

function api_export(PDO $db, array $input, string $action, bool $admin): never {
    session_write_close();
    try { json_response(export_site($db)); }
    catch (Throwable $e) { error_log($e->getMessage()); json_response(['error' => '导出失败，请检查磁盘空间、目录权限及服务日志'], 500); }
}

function api_netlify_save(PDO $db, array $input, string $action, bool $admin): never {
    $token = trim((string)($input['token'] ?? ''));
    $site = trim((string)($input['site'] ?? ''));
    if ($token === '' && $site === '') {
        $db->exec("DELETE FROM settings WHERE key IN ('netlify_token', 'netlify_site')");
        json_response(['ok' => true]);
    }
    if (!preg_match('/^[A-Za-z0-9-]+$/', $site)) json_response(['error' => '站点 ID 格式不正确'], 422);
    $db->prepare("DELETE FROM settings WHERE key = 'netlify_token'")->execute();
    if ($token !== '') save_setting($db, 'netlify_token', $token);
    save_setting($db, 'netlify_site', $site);
    json_response(['ok' => true]);
}

/** 一键部署：派生独立后台进程执行（bin/deploy-netlify.php），HTTP 立即返回——关页面/刷新不影响任务；进度走 deploy-status 轮询 */
function api_netlify_deploy(PDO $db, array $input, string $action, bool $admin): never {
    $settings = settings($db);
    if (empty($settings['netlify_site']) || empty($settings['netlify_token'])) json_response(['error' => '请先填写并保存 Netlify 令牌和站点 ID'], 422);
    $lock = fopen(DATA_DIR . '/deploy.lock', 'c');
    if (!flock($lock, LOCK_EX | LOCK_NB)) { fclose($lock); json_response(['error' => '已有部署正在进行，请等待完成后再发起'], 409); }
    $status = netlify_status_read();
    if (in_array($status['state'] ?? '', ['exporting', 'uploading', 'waiting'], true) && time() - (int)($status['updated_at'] ?? 0) < 900) {
        fclose($lock);
        json_response(['error' => '已有部署正在进行，请等待完成后再发起'], 409);
    }
    // 持锁写入启动占位；子进程再持有同一把锁直到部署结束。
    try {
        @unlink(DATA_DIR . '/deploy.cancel');
        netlify_status_write(['deploy_id' => '', 'state' => 'exporting', 'started_at' => time(), 'files_total' => 0, 'files_done' => 0, 'current' => '', 'url' => '', 'error' => '']);
        auto_deploy_pending_clear(); // 手动部署内容已是最新，取消排队中的自动任务
    } finally { fclose($lock); }
    deploy_spawn(dirname(__DIR__, 2) . '/bin/deploy-netlify.php');
    json_response(['ok' => true]);
}

function api_cancel_deploy(PDO $db, array $input, string $action, bool $admin): never {
    if (!netlify_deploy_running()) json_response(['error' => '当前没有进行中的部署'], 409);
    file_put_contents(DATA_DIR . '/deploy.cancel', '1', LOCK_EX);
    json_response(['ok' => true]);
}

/** 部署进度查询：status 为任务状态（未部署过为 null），pending 为防抖排队（含剩余秒数），前端 2s 轮询 */
function api_deploy_status(PDO $db, bool $admin): never {
    if (!$admin) json_response(['error' => '请先登录管理后台'], 401);
    $pending = auto_deploy_pending_read();
    json_response([
        'status' => netlify_status_read() ?: null,
        'cancelling' => is_file(DATA_DIR . '/deploy.cancel') && netlify_deploy_running(),
        'pending' => $pending ? ['due_at' => (int)($pending['due_at'] ?? 0), 'in' => max(0, (int)($pending['due_at'] ?? 0) - time())] : null,
    ]);
}

function api_backup_export(PDO $db, bool $admin): never {
        if (!$admin) json_response(['error' => '请先登录管理后台'], 401);
        $uploads = [];
        foreach (glob(DATA_DIR . '/uploads/*') as $f) {
            if (is_file($f)) $uploads[basename($f)] = base64_encode((string)file_get_contents($f));
        }
        json_response(['app' => 'mkblog', 'version' => 1, 'created_at' => date(DATE_ATOM), 'settings' => settings($db), 'posts' => $db->query('SELECT * FROM posts')->fetchAll(PDO::FETCH_ASSOC), 'articles' => $db->query('SELECT * FROM articles')->fetchAll(PDO::FETCH_ASSOC), 'article_revisions' => $db->query('SELECT * FROM article_revisions')->fetchAll(PDO::FETCH_ASSOC), 'tombstones' => $db->query('SELECT * FROM plugin_tombstones')->fetchAll(PDO::FETCH_ASSOC), 'comments' => $db->query('SELECT id, kind, target_id, author, email, site, content, created_at, status, reply_to FROM comments')->fetchAll(PDO::FETCH_ASSOC), 'uploads' => $uploads]);
}

/** 打包 dist/ 为 zip 供下载（须先生成静态站点；缓存 zip 比 dist 最新文件新时直接复用） */
function api_download_static(PDO $db, bool $admin): never {
    if (!$admin) json_response(['error' => '请先登录管理后台'], 401);
    $dist = dirname(__DIR__, 2) . '/dist';
    if (!is_file($dist . '/index.html')) json_response(['error' => '还没有生成静态站点，请先点「生成静态站点」'], 422);
    $zip = DATA_DIR . '/static-site.zip';
    $newest = 0;
    foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($dist, FilesystemIterator::SKIP_DOTS)) as $f) {
        if ($f->isFile()) $newest = max($newest, $f->getMTime());
    }
    if (!is_file($zip) || filemtime($zip) < $newest) {
        if (!class_exists('ZipArchive')) json_response(['error' => '服务器 PHP 缺少 zip 扩展'], 500);
        $tmp = $zip . '.tmp';
        $arch = new ZipArchive();
        if ($arch->open($tmp, ZipArchive::CREATE | ZipArchive::OVERWRITE) !== true) json_response(['error' => '压缩包创建失败'], 500);
        foreach (new RecursiveIteratorIterator(new RecursiveDirectoryIterator($dist, FilesystemIterator::SKIP_DOTS)) as $f) {
            if ($f->isFile()) $arch->addFile($f->getPathname(), substr($f->getPathname(), strlen($dist) + 1));
        }
        $arch->close();
        rename($tmp, $zip);
    }
    session_write_close();
    header('Content-Type: application/zip');
    header('Content-Disposition: attachment; filename="muke-static-site.zip"');
    header('Content-Length: ' . filesize($zip));
    header('X-Content-Type-Options: nosniff');
    readfile($zip);
    exit;
}

function api_backup_import(PDO $db, array $input, string $action, bool $admin): never {
    if (!isset($input['app']) || $input['app'] !== 'mkblog') json_response(['error' => '备份文件格式不正确'], 422);
    $restoredFiles = [];
    $db->beginTransaction();
    try {
        $db->exec('DELETE FROM posts');
        $db->exec('DELETE FROM articles');
        $db->exec('DELETE FROM article_revisions');
        $db->exec('DELETE FROM comments');
        $db->exec('DELETE FROM plugin_tombstones');
        $db->exec('DELETE FROM settings');
        $insSetting = $db->prepare('INSERT INTO settings (key, value) VALUES (?, ?)');
        foreach ((array)($input['settings'] ?? []) as $k => $v) $insSetting->execute([(string)$k, (string)$v]);
        $insPost = $db->prepare('INSERT INTO posts (id, content, images, meta, created_at, updated_at, source, source_key, link, pinned) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)');
        foreach ((array)($input['posts'] ?? []) as $row) $insPost->execute([$row['id'] ?? null, $row['content'] ?? '', $row['images'] ?? '[]', $row['meta'] ?? '', $row['created_at'] ?? date(DATE_ATOM), $row['updated_at'] ?? date(DATE_ATOM), $row['source'] ?? '', $row['source_key'] ?? '', $row['link'] ?? '', empty($row['pinned']) ? 0 : 1]);
        $insArticle = $db->prepare('INSERT INTO articles (id, title, content, format, tags, draft, images, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)');
        foreach ((array)($input['articles'] ?? []) as $row) $insArticle->execute([$row['id'] ?? null, $row['title'] ?? '', $row['content'] ?? '', $row['format'] ?? 'md', $row['tags'] ?? '', (int)($row['draft'] ?? 0), $row['images'] ?? '[]', $row['created_at'] ?? date(DATE_ATOM), $row['updated_at'] ?? date(DATE_ATOM)]);
        $insTomb = $db->prepare('INSERT OR IGNORE INTO plugin_tombstones (key, plugin, deleted_at) VALUES (?, ?, ?)');
        $insRevision = $db->prepare('INSERT INTO article_revisions (id, article_id, title, content, format, tags, draft, images, updated_at, saved_at) SELECT ?, id, ?, ?, ?, ?, ?, ?, ?, ? FROM articles WHERE id = ?');
        foreach ((array)($input['article_revisions'] ?? []) as $row) {
            $content = (string)($row['content'] ?? '');
            $insRevision->execute([$row['id'] ?? null, (string)($row['title'] ?? ''), $content, ($row['format'] ?? '') === 'html' ? 'html' : 'md', normalize_tags((string)($row['tags'] ?? '')), empty($row['draft']) ? 0 : 1, json_encode(article_image_refs($content)), $row['updated_at'] ?? date(DATE_ATOM), $row['saved_at'] ?? date(DATE_ATOM), (int)($row['article_id'] ?? 0)]);
        }
        foreach ((array)($input['tombstones'] ?? []) as $row) $insTomb->execute([$row['key'] ?? '', $row['plugin'] ?? '', $row['deleted_at'] ?? date(DATE_ATOM)]);
        $insComment = $db->prepare('INSERT INTO comments (id, kind, target_id, author, email, site, content, created_at, status, reply_to) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)');
        foreach ((array)($input['comments'] ?? []) as $row) $insComment->execute([$row['id'] ?? null, in_array($row['kind'] ?? '', COMMENT_KINDS, true) ? $row['kind'] : 'post', (int)($row['target_id'] ?? 0), mb_substr((string)($row['author'] ?? ''), 0, 40), (string)($row['email'] ?? ''), (string)($row['site'] ?? ''), (string)($row['content'] ?? ''), $row['created_at'] ?? date(DATE_ATOM), in_array($row['status'] ?? '', ['approved', 'pending', 'spam'], true) ? $row['status'] : 'approved', $row['reply_to'] ?? null]);
        foreach ((array)($input['uploads'] ?? []) as $name => $b64) {
            if (!preg_match('/^[a-f0-9]{32}\.(jpg|png|webp|gif|avif)$/', (string)$name)) continue;
            $data = base64_decode((string)$b64, true);
            if ($data === false) continue;
            if (!is_dir(DATA_DIR . '/uploads')) mkdir(DATA_DIR . '/uploads', 0755, true);
            $path = DATA_DIR . '/uploads/' . $name;
            $staged = tempnam(DATA_DIR . '/uploads', '.restore-');
            if ($staged === false) throw new RuntimeException('无法创建恢复临时文件');
            try {
                if (file_put_contents($staged, $data) !== strlen($data)) throw new RuntimeException('写入恢复图片失败');
                $previous = null;
                if (is_file($path)) {
                    $previous = $staged . '.previous';
                    if (!rename($path, $previous)) throw new RuntimeException('暂存原图片失败');
                }
                $restoredFiles[$path] = $previous;
                if (!rename($staged, $path)) throw new RuntimeException('恢复图片失败');
            } finally { if (is_file($staged)) unlink($staged); }
        }
        $db->commit();
        foreach ($restoredFiles as $previous) if ($previous !== null) unlink($previous);
        json_response(['ok' => true, 'posts' => count((array)($input['posts'] ?? [])), 'articles' => count((array)($input['articles'] ?? []))]);
    } catch (Throwable $e) {
        if ($db->inTransaction()) $db->rollBack();
        foreach ($restoredFiles as $path => $previous) {
            if (is_file($path)) unlink($path);
            if ($previous !== null) rename($previous, $path);
        }
        json_response(['error' => '导入失败：' . $e->getMessage()], 500);
    }
}
