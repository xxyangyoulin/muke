<?php
declare(strict_types=1);

// 壁纸收藏插件（内置，自包含）：扫描本地壁纸目录 → 瀑布流页面（plugin.js/plugin.css）+ 新壁纸自动发动态。
// 本文件提供：表结构（tables）、页面声明（page）、图库数据端点（data）、图片引用解析（resolve_ref）、
// 图片服务（image_endpoint）、静态导出（export）、同步（sync）。核心层不包含任何壁纸代码。

$wall_dir = fn(): string => rtrim((string)opt('wallpaper_dir'), '/');

$wall_enabled = fn(): bool => filter_var(opt('wallpapers_enabled'), FILTER_VALIDATE_BOOLEAN);

$wall_forget = function (PDO $db, string $id): array {
    $ownTransaction = !$db->inTransaction();
    if ($ownTransaction) $db->beginTransaction();
    try {
        $ref = 'wall:' . $id;
        $stmt = $db->prepare('SELECT id, images FROM posts WHERE images LIKE ?');
        $stmt->execute(['%"' . $ref . '"%']);
        $deleted = [];
        foreach ($stmt->fetchAll(PDO::FETCH_ASSOC) as $post) {
            $images = array_values(array_diff(json_decode($post['images'], true), [$ref]));
            if ($images) $db->prepare('UPDATE posts SET images = ? WHERE id = ?')->execute([json_encode($images), $post['id']]);
            else {
                $db->prepare("DELETE FROM comments WHERE kind = 'post' AND target_id = ?")->execute([$post['id']]);
                $db->prepare('DELETE FROM post_likes WHERE post_id = ?')->execute([$post['id']]);
                $db->prepare('DELETE FROM posts WHERE id = ?')->execute([$post['id']]);
                $deleted[] = (int)$post['id'];
            }
        }
        $db->prepare('DELETE FROM wallpapers WHERE id = ?')->execute([$id]);
        if ($ownTransaction) $db->commit();
        return $deleted;
    } catch (Throwable $e) {
        if ($ownTransaction && $db->inTransaction()) $db->rollBack();
        throw $e;
    }
};

// 扫描目录：图片文件（跳过隐藏文件）→ 按 mtime 倒序
$wall_scan = function () use ($wall_dir): array {
    $dir = $wall_dir();
    if ($dir === '' || !is_dir($dir) || !is_readable($dir)) throw new RuntimeException('壁纸目录不可读取，本次同步已取消');
    $items = [];
    $files = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($dir, FilesystemIterator::SKIP_DOTS));
    foreach ($files as $file) {
        if (!$file->isFile() || $file->isLink()) continue;
        if (str_starts_with($file->getBasename(), '.')) continue; // 隐藏文件（如 .trashed- 改名）不入图库
        $path = $file->getPathname();
        $size = @getimagesize($path);
        if (!$size || !in_array($size['mime'], IMAGE_MIMES, true)) continue;
        $relative = substr($path, strlen($dir) + 1);
        $items[] = ['id' => hash('sha256', $relative), 'name' => $file->getBasename(), 'path' => $relative, 'width' => $size[0], 'height' => $size[1], 'mtime' => $file->getMTime(), 'bytes' => $file->getSize()];
    }
    usort($items, fn($a, $b) => $b['mtime'] <=> $a['mtime'] ?: strcmp($a['name'], $b['name']));
    return $items;
};

// 扫描结果与 wallpapers 表对齐：新文件登记、改动更新、消失的连带删动态（不立墓碑，文件回归可重新发布）
$wall_sync = function (PDO $db) use ($wall_scan, $wall_enabled, $wall_forget): array {
    if (!$wall_enabled()) return [];
    $lock = @fopen(DATA_DIR . '/wallpaper.lock', 'c');
    if ($lock && !flock($lock, LOCK_EX | LOCK_NB)) { fclose($lock); return $wall_scan(); }
    try {
        $scanned = $wall_scan();
        $known = [];
        foreach ($db->query('SELECT id, mtime, post_id FROM wallpapers')->fetchAll(PDO::FETCH_ASSOC) as $row) $known[$row['id']] = $row;
        $insert = $db->prepare('INSERT INTO wallpapers (id, path, name, width, height, bytes, mtime) VALUES (?, ?, ?, ?, ?, ?, ?)');
        $update = $db->prepare('UPDATE wallpapers SET path = ?, name = ?, width = ?, height = ?, bytes = ?, mtime = ? WHERE id = ?');
        foreach ($scanned as $wall) {
            if (!isset($known[$wall['id']])) {
                $insert->execute([$wall['id'], $wall['path'], $wall['name'], $wall['width'], $wall['height'], $wall['bytes'], $wall['mtime']]);
            } elseif ((int)$known[$wall['id']]['mtime'] !== $wall['mtime']) {
                $update->execute([$wall['path'], $wall['name'], $wall['width'], $wall['height'], $wall['bytes'], $wall['mtime'], $wall['id']]);
            }
        }
        $present = array_fill_keys(array_column($scanned, 'id'), true);
        foreach ($known as $id => $row) {
            if (isset($present[$id])) continue;
            // 批量动态保留其余图片；最后一张消失时才删除动态。
            $wall_forget($db, $id);
        }
        return $scanned;
    } finally {
        if ($lock) { flock($lock, LOCK_UN); fclose($lock); }
    }
};

return [
    'id' => 'wallpapers',
    'name' => '壁纸收藏',
    'schema_version' => 1,
    'ref_protocols' => ['wall'],
    'backup_paths' => fn(): array => ['wallpaper_dir' => $wall_dir()],
    'defaults' => ['wallpapers_enabled' => true, 'wallpaper_dir' => '', 'wallpaper_sync_interval' => 30, 'wallpaper_post_template' => '收了一张新壁纸：{name}', 'gallery_batch' => 24],
    'env' => ['MKBLOG_WALLPAPERS_ENABLED' => 'wallpapers_enabled', 'MKBLOG_WALLPAPER_DIR' => 'wallpaper_dir', 'MKBLOG_SYNC_INTERVAL' => 'wallpaper_sync_interval', 'MKBLOG_WALLPAPER_POST' => 'wallpaper_post_template', 'MKBLOG_GALLERY_BATCH' => 'gallery_batch'],
    'description' => '扫描本地壁纸目录并展示瀑布流；新壁纸按文件时间自动发布为一条动态。',
    'setting_keys' => [
        'enabled' => 'wallpapers_enabled',
        'wallpaper_dir' => 'wallpaper_dir',
        'wallpaper_sync_interval' => 'wallpaper_sync_interval',
        'wallpaper_post_template' => 'wallpaper_post_template',
        'gallery_batch' => 'gallery_batch',
    ],
    'fields' => [
        ['key' => 'wallpaper_dir', 'label' => '壁纸目录', 'type' => 'text', 'pattern' => '#^/#', 'hint' => '绝对路径（以 / 开头）；留空且未配置时不展示壁纸页'],
        ['key' => 'wallpaper_sync_interval', 'label' => '自动同步间隔（秒）', 'type' => 'number', 'min' => 0, 'max' => 86400, 'default' => 30, 'hint' => '0 = 仅手动同步'],
        ['key' => 'wallpaper_post_template', 'label' => '新壁纸动态模板', 'type' => 'text', 'max' => 200, 'hint' => '{name} 为文件名，留空不发动态'],
        ['key' => 'gallery_batch', 'label' => '瀑布流每批加载（张）', 'type' => 'number', 'min' => 6, 'max' => 96, 'default' => 24],
    ],
    'interval_key' => 'wallpaper_sync_interval',
    'interval_unit' => 'seconds',
    // 壁纸动态的图片保持原始尺寸（不套用普通动态单图 50% 宽的紧凑样式）
    'post_images_full' => true,

    // 插件自有表结构 + 历史数据迁移（核心 app.php 循环调用）
    'tables' => function (PDO $db): void {
        $db->exec('CREATE TABLE IF NOT EXISTS wallpapers (id TEXT PRIMARY KEY, path TEXT NOT NULL, name TEXT NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL, bytes INTEGER NOT NULL, mtime INTEGER NOT NULL, post_id INTEGER)');
        // 旧版本数据：把带 wall: 引用却无来源标记的动态归到本插件名下
        foreach ($db->query("SELECT id, images FROM posts WHERE source = '' AND images LIKE '%wall:%'")->fetchAll(PDO::FETCH_ASSOC) as $row) {
            foreach (json_decode($row['images'], true) ?: [] as $ref) {
                if (is_string($ref) && str_starts_with($ref, 'wall:')) {
                    $db->prepare('UPDATE posts SET source = \'wallpapers\', source_key = ? WHERE id = ?')->execute([$ref, $row['id']]);
                    break;
                }
            }
        }
    },

    // 声明公开页面：核心据此放行路由、渲染导航、导出静态页
    'page' => ['route' => '/wallpapers', 'title' => '壁纸收藏', 'description' => '壁纸收藏——精选风景壁纸，可在线预览与下载原图。', 'nav' => ['icon' => 'image', 'label' => '壁纸收藏', 'count' => true]],

    'sync' => function (PDO $db, array $config) use ($wall_sync): array {
        // 扫描 + 登记 + 清理（文件删除的动态在此被删，不立墓碑——文件回来可重新发动态）
        $scanned = $wall_sync($db);
        $template = (string)($config['wallpaper_post_template'] ?? '');
        if ($template === '') return [];
        if (!isset(settings($db)['plugin:wallpapers:backfilled'])) {
            // 首次启用：已有图库只登记不发布，避免刷屏
            save_setting($db, 'plugin:wallpapers:backfilled', '1');
            $db->exec('UPDATE wallpapers SET post_id = 1');
            return [];
        }
        $tombstoned = array_fill_keys($db->query("SELECT key FROM plugin_tombstones WHERE plugin = 'wallpapers'")->fetchAll(PDO::FETCH_COLUMN), true);
        $posted = array_flip($db->query('SELECT id FROM wallpapers WHERE post_id IS NOT NULL')->fetchAll(PDO::FETCH_COLUMN));
        $items = [];
        foreach ($scanned as $wall) {
            $key = 'wall:' . $wall['id'];
            if (isset($tombstoned[$key]) || isset($posted[$wall['id']])) continue;
            $items[] = [
                'key' => $key,
                'content' => str_replace('{name}', $wall['name'], $template),
                'created_at' => date(DATE_ATOM, $wall['mtime']),
                'images' => [$key],
            ];
        }
        return $items;
    },
    'after_sync' => function (PDO $db, array $items): void {
        $mark = $db->prepare('UPDATE wallpapers SET post_id = 1 WHERE id = ?');
        foreach ($items as $item) $mark->execute([substr((string)($item['key'] ?? ''), 5)]);
    },

    // 图库数据端点（公开 GET ?action=plugin-data&id=wallpapers）
    'data' => function (PDO $db, bool $admin) use ($wall_dir, $wall_enabled): array {
        $dir = $wall_dir();
        return [
            'items' => $db->query('SELECT id, path, name, width, height, bytes, mtime FROM wallpapers ORDER BY mtime DESC, name')->fetchAll(PDO::FETCH_ASSOC),
            'available' => $wall_enabled() && $dir !== '' && is_readable($dir),
            'batch' => (int)opt('gallery_batch'),
            'interval' => (int)opt('wallpaper_sync_interval'),
        ];
    },

    // 用户删除壁纸动态：墓碑已阻止重发，无需额外清理（post_id 是发布标记而非外键）

    // wall:<id> 引用 → 动态图片描述符（核心 post_images 对非 up: 引用调用）
    'resolve_ref' => function (PDO $db, string $ref): ?array {
        if (!str_starts_with($ref, 'wall:')) return null;
        $stmt = $db->prepare('SELECT path, name, width, height, bytes, mtime FROM wallpapers WHERE id = ?');
        $stmt->execute([substr($ref, 5)]);
        if (!$wall = $stmt->fetch(PDO::FETCH_ASSOC)) return null;
        $file = '/image?file=' . rawurlencode($wall['path']) . '&v=' . $wall['mtime'];
        return ['url' => $file, 'thumb' => $file . '&thumb=1', 'name' => $wall['name'], 'width' => (int)$wall['width'], 'height' => (int)$wall['height'], 'bytes' => (int)$wall['bytes']];
    },

    // /image?file=<相对路径> → 壁纸目录内文件（含越界防护）
    'image_endpoint' => function (string $relative) use ($wall_dir): ?string {
        $dir = $wall_dir();
        if ($dir === '' || !is_string($relative) || str_contains($relative, "\0")) return null;
        $path = realpath($dir . '/' . $relative);
        $root = realpath($dir);
        if (!$path || !$root || !str_starts_with($path, $root . '/') || !is_file($path)) return null;
        return $path;
    },

    // 管理操作（仅管理员）：上传原图或删除壁纸（源文件 + 登记 + 连带动态）
    'admin_op' => function (PDO $db, string $op, array $input, array $config) use ($wall_dir, $wall_forget) {
        if ($op === 'upload') {
            if (empty($config['_enabled'])) throw new InvalidArgumentException('插件未启用');
            $dir = $wall_dir();
            if ($dir === '' || !is_dir($dir)) throw new InvalidArgumentException('请先配置有效的壁纸目录');
            $files = $_FILES['images'] ?? [];
            if (empty($files['error']) || !is_array($files['error'])) throw new InvalidArgumentException('请选择要上传的图片');
            if (count($files['error']) !== (int)($input['count'] ?? 0)) throw new InvalidArgumentException('上传数量超过服务器限制，请减少图片数量后重试');
            $maxMb = (int)opt('upload_max_mb');
            $prepared = [];
            foreach ($files['error'] as $i => $error) {
                if ($error !== UPLOAD_ERR_OK) throw new InvalidArgumentException('图片上传失败，请检查文件大小或重试');
                if ($files['size'][$i] > $maxMb * 1024 * 1024) throw new InvalidArgumentException("单张图片不能超过 {$maxMb}MB");
                $info = @getimagesize($files['tmp_name'][$i]);
                if (!$info || !in_array($info['mime'], IMAGE_MIMES, true)) throw new InvalidArgumentException('仅支持 JPEG / PNG / WebP / GIF / AVIF 图片');
                $relative = $info[0] > $info[1] ? '' : 'Portrait/';
                $targetDir = $dir . '/' . $relative;
                if (!is_dir($targetDir) && !@mkdir($targetDir, 0755, true)) throw new InvalidArgumentException('无法创建 Portrait 目录，请检查目录权限');
                $root = realpath($dir);
                $target = realpath($targetDir);
                if ($target !== $root && !str_starts_with($target . '/', $root . '/')) throw new InvalidArgumentException('上传目录必须位于壁纸目录内');
                $stem = trim(preg_replace('/[\x00-\x1f\x7f\/\\\\:*?"<>|]/u', '_', pathinfo($files['name'][$i], PATHINFO_FILENAME)), '. ');
                $prepared[$i] = ['prefix' => $relative, 'stem' => mb_substr($stem ?: 'wallpaper', 0, 80), 'info' => $info];
            }
            // 与定时同步共用锁，避免一批图片在上传途中被拆成多条动态。
            $syncLock = fopen(DATA_DIR . '/plugin.sync.lock', 'c');
            flock($syncLock, LOCK_EX);
            $wallLock = fopen(DATA_DIR . '/wallpaper.lock', 'c');
            flock($wallLock, LOCK_EX);
            $uploaded = [];
            $events = [];
            try {
                $db->beginTransaction();
                $insert = $db->prepare('INSERT INTO wallpapers (id, path, name, width, height, bytes, mtime, post_id) VALUES (?, ?, ?, ?, ?, ?, ?, 1)');
                $refs = [];
                foreach ($prepared as $i => $item) {
                    $extension = '.' . MIME_EXT[$item['info']['mime']];
                    $name = $item['stem'] . $extension;
                    if (file_exists($dir . '/' . $item['prefix'] . $name)) $name = $item['stem'] . '-' . bin2hex(random_bytes(8)) . $extension;
                    $relative = $item['prefix'] . $name;
                    if (!move_uploaded_file($files['tmp_name'][$i], $dir . '/' . $relative)) throw new InvalidArgumentException('壁纸保存失败，请检查目录权限');
                    $uploaded[] = $relative;
                    $id = hash('sha256', $relative);
                    $insert->execute([$id, $relative, $name, $item['info'][0], $item['info'][1], $files['size'][$i], filemtime($dir . '/' . $relative)]);
                    $refs[] = 'wall:' . $id;
                }
                $template = (string)($config['wallpaper_post_template'] ?? '');
                if ($template !== '') {
                    $content = count($uploaded) === 1 ? str_replace('{name}', basename($uploaded[0]), $template) : '收了 ' . count($uploaded) . ' 张新壁纸';
                    plugin_sync_items($db, 'wallpapers', [['key' => 'wall-upload:' . bin2hex(random_bytes(16)), 'content' => $content, 'images' => $refs]], $events);
                }
                $db->commit();
            } catch (Throwable $e) {
                if ($db->inTransaction()) $db->rollBack();
                foreach ($uploaded as $relative) unlink($dir . '/' . $relative);
                throw $e;
            } finally {
                flock($wallLock, LOCK_UN); fclose($wallLock);
                flock($syncLock, LOCK_UN); fclose($syncLock);
            }
            foreach ($events as $event) plugin_fire('post.saved', $event);
            return ['uploaded' => $uploaded];
        }
        if ($op !== 'delete') throw new InvalidArgumentException('操作无效');
        $wallId = (string)($input['wall_id'] ?? '');
        if (!preg_match('/^[a-f0-9]{64}$/', $wallId)) throw new InvalidArgumentException('壁纸标识无效');
        $syncLock = fopen(DATA_DIR . '/plugin.sync.lock', 'c');
        $wallLock = fopen(DATA_DIR . '/wallpaper.lock', 'c');
        flock($syncLock, LOCK_EX);
        flock($wallLock, LOCK_EX);
        $staged = null;
        try {
            $db->beginTransaction();
            $stmt = $db->prepare('SELECT path, name, post_id FROM wallpapers WHERE id = ?');
            $stmt->execute([$wallId]);
            $wall = $stmt->fetch(PDO::FETCH_ASSOC);
            if (!$wall) throw new InvalidArgumentException('壁纸不存在');
            $dir = $wall_dir();
            $path = realpath($dir . '/' . $wall['path']);
            $root = realpath($dir);
            if ($path) {
                if (!$root || !str_starts_with($path, $root . '/') || !is_file($path)) throw new InvalidArgumentException('壁纸路径无效');
                $staged = dirname($path) . '/.deleted-' . bin2hex(random_bytes(16));
                if (!rename($path, $staged)) throw new InvalidArgumentException('文件删除失败（权限不足？）');
            }
            $deletedPosts = $wall_forget($db, $wallId);
            $db->commit();
        } catch (Throwable $e) {
            if ($db->inTransaction()) $db->rollBack();
            if ($staged !== null && is_file($staged)) rename($staged, $path);
            throw $e;
        } finally {
            flock($wallLock, LOCK_UN); fclose($wallLock);
            flock($syncLock, LOCK_UN); fclose($syncLock);
        }
        if ($staged !== null) unlink($staged);
        return ['deleted' => $wall['name'], 'deleted_posts' => $deletedPosts];
    },

    // 静态导出：复制原图+缩略图、生成引用映射与无 JS 图库页
    'export' => function (PDO $db, string $dest, array &$files, bool $enabled) use ($wall_dir): array {
        $escape = fn($s) => htmlspecialchars((string)$s, ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
        $walls = $db->query($enabled ? 'SELECT * FROM wallpapers ORDER BY mtime DESC, name' : "SELECT w.* FROM wallpapers w WHERE EXISTS (SELECT 1 FROM posts p WHERE p.source = 'wallpapers' AND p.images LIKE '%\"wall:' || w.id || '\"%')")->fetchAll(PDO::FETCH_ASSOC);
        $walls = array_values(array_filter($walls, fn($wall) => is_file($wall_dir() . '/' . $wall['path'])));
        $wallDir = $wall_dir();
        $refs = [];
        foreach ($walls as &$wall) {
            $source = $wallDir . '/' . $wall['path'];
            $info = getimagesize($source);
            $name = $wall['id'] . '-' . $wall['mtime'];
            $wall['original'] = '/media/' . $name . '.' . MIME_EXT[$info['mime']];
            $wall['thumbnail'] = '/thumbnails/' . $name . '.webp';
            $original = $dest . $wall['original'];
            $thumbnail = $dest . $wall['thumbnail'];
            if (!is_file($original) && !copy($source, $original)) throw new RuntimeException('壁纸原图复制失败');
            if (!is_file($thumbnail)) {
                exec(thumb_magick($source, $thumbnail), $output, $code);
                if ($code !== 0) throw new RuntimeException('壁纸缩略图生成失败，需要 ImageMagick');
            }
            $files[] = $original;
            $files[] = $thumbnail;
            $refs['wall:' . $wall['id']] = ['url' => $wall['original'], 'thumb' => $wall['thumbnail'], 'name' => $wall['name'], 'width' => $wall['width'], 'height' => $wall['height'], 'bytes' => $wall['bytes']];
            unset($wall['path']);
        }
        unset($wall);
        $body = '<div class="gallery-hero"><h1>壁纸收藏。</h1><p>精选风景已随站点打包，可在线预览、下载原图。</p></div><div class="masonry-fallback">';
        foreach ($walls as $wall) {
            $body .= '<article class="media-tile"><a class="tile-preview" href="' . $escape($wall['original']) . '" aria-label="查看 ' . $escape($wall['name']) . '"><img src="' . $escape($wall['thumbnail']) . '" alt="' . $escape($wall['name']) . '" loading="lazy"></a></article>';
        }
        $body .= '</div>';
        return ['items' => $walls, 'refs' => $refs, 'html' => $body, 'meta' => ['batch' => (int)opt('gallery_batch')]];
    },
];
