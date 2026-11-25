<?php
declare(strict_types=1);

/** 部署任务状态（var/deploy.status.json）：后台任务进程写入、API 与前端轮询读取，文件即进度 */
function netlify_status_path(): string { return DATA_DIR . '/deploy.status.json'; }

function netlify_status_read(): array {
    $data = is_file(netlify_status_path()) ? json_decode((string)file_get_contents(netlify_status_path()), true) : [];
    return is_array($data) ? $data : [];
}

/** 合并写入并盖 updated_at 心跳；state: exporting → uploading → waiting → done | error | cancelled */
function netlify_status_write(array $patch): array {
    $data = array_merge(netlify_status_read(), $patch, ['updated_at' => time()]);
    file_put_contents(netlify_status_path(), json_encode($data, JSON_UNESCAPED_UNICODE), LOCK_EX);
    return $data;
}

class DeployCancelled extends RuntimeException {}

function netlify_check_cancel(): void {
    clearstatcache(true, DATA_DIR . '/deploy.cancel');
    if (is_file(DATA_DIR . '/deploy.cancel')) throw new DeployCancelled('部署已取消');
}

/** 15 分钟无心跳的「进行中」视为僵死，允许重新发起 */
function netlify_deploy_running(): bool {
    $lock = fopen(DATA_DIR . '/deploy.lock', 'c');
    $busy = !flock($lock, LOCK_EX | LOCK_NB);
    fclose($lock);
    if ($busy) return true;
    $status = netlify_status_read();
    return in_array($status['state'] ?? '', ['exporting', 'uploading', 'waiting'], true)
        && time() - (int)($status['updated_at'] ?? 0) < 900;
}

/** 派生独立后台 PHP 进程：关闭全部继承 fd（含连接/监听 socket，不攥端口；响应靠 Content-Length 自定界），退出码恒 0 */
function deploy_spawn(string $script): void {
    $root = dirname(__DIR__);
    $cmd = sprintf(<<<'SH'
cd %s && { for n in /proc/self/fd/*; do fd=${n##*/}; [ "$fd" -gt 2 ] && eval "exec $fd>&-"; done; exec %s %s >> %s 2>&1 < /dev/null; } &
SH
        , escapeshellarg($root), escapeshellarg(PHP_SAPI === 'fpm-fcgi' ? PHP_BINDIR . '/php' : PHP_BINARY), escapeshellarg($script), escapeshellarg(DATA_DIR . '/deploy.log'));
    exec($cmd);
}

/** 执行完整部署（导出 → 摘要上传 → 状态落盘），后台手动部署与自动部署共用；返回是否成功 */
function netlify_deploy_run(PDO $db, ?array $pending = null): bool {
    $lock = fopen(DATA_DIR . '/deploy.lock', 'c');
    if (!flock($lock, LOCK_EX | LOCK_NB)) {
        fclose($lock);
        return false;
    }
    try {
        if ($pending !== null || !in_array(netlify_status_read()['state'] ?? '', ['exporting', 'uploading', 'waiting'], true)) @unlink(DATA_DIR . '/deploy.cancel');
        netlify_check_cancel();
        netlify_status_write(['deploy_id' => '', 'state' => 'exporting', 'started_at' => time(), 'files_total' => 0, 'files_done' => 0, 'current' => '', 'url' => '', 'error' => '']);
        echo '导出静态站点…' . PHP_EOL;
        $result = export_site($db, 'netlify_check_cancel');
        netlify_check_cancel();
        netlify_status_write(['posts' => (int)$result['posts'], 'plugins' => $result['plugins']]);
        echo "已导出 {$result['posts']} 条动态及插件内容，开始上传…" . PHP_EOL;
        $settings = settings($db);
        $url = netlify_deploy((string)$settings['netlify_token'], (string)$settings['netlify_site'], fn(array $patch) => netlify_status_write($patch));
        netlify_check_cancel();
        netlify_status_write(['state' => 'done', 'url' => $url]);
        echo "部署完成：{$url}" . PHP_EOL;
        return true;
    } catch (DeployCancelled $e) {
        $deployId = netlify_status_read()['deploy_id'] ?? '';
        $error = '';
        if ($deployId !== '') {
            try {
                [$status] = netlify_request((string)opt('netlify_token'), 'POST', 'https://api.netlify.com/api/v1/deploys/' . rawurlencode($deployId) . '/cancel', null, 'application/json', 15, false);
                if ($status >= 400) $error = '本地部署已停止，远端取消未成功，请检查 Netlify 状态';
            } catch (Throwable $remoteError) { $error = '本地部署已停止，远端取消失败：' . $remoteError->getMessage(); }
        }
        netlify_status_write(['state' => 'cancelled', 'current' => '', 'error' => $error]);
        echo '部署已取消' . PHP_EOL;
        return false;
    } catch (Throwable $e) {
        netlify_status_write(['state' => 'error', 'error' => $e->getMessage()]);
        fwrite(STDERR, '部署失败：' . $e->getMessage() . PHP_EOL);
        return false;
    } finally {
        if ($pending !== null) auto_deploy_pending_clear($pending);
        flock($lock, LOCK_UN);
        fclose($lock);
    }
}

/** ---- 自动部署（防抖）---- pending 文件记录到期时间，新动态不断顺延；到期由 bin/auto-deploy.php 执行 ---- */

function auto_deploy_pending_path(): string { return DATA_DIR . '/deploy.pending.json'; }

function auto_deploy_pending_read(): array {
    $lock = fopen(DATA_DIR . '/deploy.pending.lock', 'c');
    flock($lock, LOCK_SH);
    $data = is_file(auto_deploy_pending_path()) ? json_decode((string)file_get_contents(auto_deploy_pending_path()), true) : [];
    fclose($lock);
    return is_array($data) ? $data : [];
}

function auto_deploy_pending_clear(?array $expected = null): void {
    $lock = fopen(DATA_DIR . '/deploy.pending.lock', 'c');
    flock($lock, LOCK_EX);
    $current = is_file(auto_deploy_pending_path()) ? json_decode((string)file_get_contents(auto_deploy_pending_path()), true) : [];
    if ($expected === null || $current === $expected) @unlink(auto_deploy_pending_path());
    fclose($lock);
}

/** 新动态触碰：开关开启且凭据齐备时顺延到期时间，并确保 watcher 存活（编辑动态不触发） */
function auto_deploy_touch(PDO $db): void {
    if (opt('auto_deploy') !== '1' || (string)opt('netlify_token') === '' || (string)opt('netlify_site') === '') return;
    $minutes = max(1, min(1440, (int)opt('auto_deploy_debounce')));
    $lock = fopen(DATA_DIR . '/deploy.pending.lock', 'c');
    flock($lock, LOCK_EX);
    file_put_contents(auto_deploy_pending_path(), json_encode(['due_at' => time() + $minutes * 60, 'touched_at' => time(), 'id' => bin2hex(random_bytes(16))]));
    fclose($lock);
    if (!auto_deploy_watcher_alive()) deploy_spawn(dirname(__DIR__) . '/bin/auto-deploy.php');
}

/** watcher 存活检查：pid 文件 + /proc 命令行校验（防 pid 复用误判） */
function auto_deploy_watcher_alive(): bool {
    $pid = (int)@file_get_contents(DATA_DIR . '/deploy.watcher.pid');
    if ($pid <= 0 || !is_dir("/proc/$pid")) return false;
    $cmdline = (string)@file_get_contents("/proc/$pid/cmdline");
    return $cmdline !== '' && str_contains($cmdline, 'auto-deploy');
}

/** 传输层失败重试 6 次；GET/PUT 幂等，HTTP 5xx 亦重试（大文件长传时 Netlify 偶发 502/504） */
function netlify_request(string $token, string $method, string $url, ?string $body = null, string $contentType = 'application/json', int $timeout = 600, bool $cancellable = true): array {
    for ($attempt = 1; ; $attempt++) {
        if ($cancellable) netlify_check_cancel();
        $ch = curl_init($url);
        $headers = ['Authorization: Bearer ' . $token];
        if ($body !== null) $headers[] = 'Content-Type: ' . $contentType;
        $options = [CURLOPT_CUSTOMREQUEST => $method, CURLOPT_RETURNTRANSFER => true, CURLOPT_HTTPHEADER => $headers, CURLOPT_POSTFIELDS => $body ?? '', CURLOPT_TIMEOUT => $timeout];
        if ($cancellable) {
            $options[CURLOPT_NOPROGRESS] = false;
            $options[CURLOPT_XFERINFOFUNCTION] = static function (): int {
                clearstatcache(true, DATA_DIR . '/deploy.cancel');
                return is_file(DATA_DIR . '/deploy.cancel') ? 1 : 0;
            };
        }
        $proxy = (string)config('http_proxy');
        if ($proxy !== '') $options[CURLOPT_PROXY] = $proxy;
        curl_setopt_array($ch, $options);
        $response = curl_exec($ch);
        $status = (int)curl_getinfo($ch, CURLINFO_RESPONSE_CODE);
        $error = curl_error($ch);
        curl_close($ch);
        if ($cancellable && !($method === 'POST' && $response !== false && $status >= 200 && $status < 300)) netlify_check_cancel();
        if ($response !== false && $status > 0 && ($status < 500 || $method === 'POST' || $attempt >= 6)) {
            $data = json_decode((string)$response, true);
            return [$status, is_array($data) ? $data : []];
        }
        if ($response === false && $attempt >= 6) throw new RuntimeException('无法连接 Netlify：' . ($error ?: '网络异常'));
        for ($wait = 0; $wait < min($attempt * 3, 15); $wait++) {
            if ($cancellable) netlify_check_cancel();
            sleep(1);
        }
    }
}

/** 部署 dist/ 到 Netlify（摘要式增量：只传 required 里的文件）。$progress 收到 ['state'=>…,'files_done'=>n,'files_total'=>m] 用于任务进度 */
function netlify_deploy(string $token, string $site, ?callable $progress = null): string {
    $dist = dirname(__DIR__) . '/dist';
    $files = [];
    $iterator = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($dist, FilesystemIterator::SKIP_DOTS));
    foreach ($iterator as $file) if ($file->isFile()) $files['/' . $iterator->getSubPathname()] = sha1_file($file->getPathname());
    if (!$files) throw new RuntimeException('dist 目录为空，请先导出静态站点');
    [$status, $deploy] = netlify_request($token, 'POST', "https://api.netlify.com/api/v1/sites/{$site}/deploys", json_encode(['files' => $files]));
    if ($status >= 400) throw new RuntimeException($deploy['message'] ?? "创建部署失败（HTTP {$status}）");
    if ($progress) $progress(['deploy_id' => $deploy['id']]);
    netlify_check_cancel();
    $required = array_fill_keys($deploy['required'] ?? [], true);
    $total = count(array_filter($files, fn($sha1) => isset($required[$sha1])));
    if ($total === 0) {
        if ($progress) $progress(['state' => 'waiting', 'files_total' => 0, 'files_done' => 0, 'current' => '']);
    } else {
        $done = 0;
        if ($progress) $progress(['state' => 'uploading', 'files_total' => $total, 'files_done' => 0, 'current' => '']);
        foreach ($files as $path => $sha1) {
            if (!isset($required[$sha1])) continue;
            $encoded = '/' . implode('/', array_map('rawurlencode', explode('/', ltrim($path, '/'))));
            // 超时按体积自适应：600s 基础 + 每 25KB/s 兜底上行速率另计（30MB 原图 ≈ 20 分钟），避免慢网络大文件必挂
            $size = (int)filesize($dist . $path);
            [$status] = netlify_request($token, 'PUT', "https://api.netlify.com/api/v1/deploys/{$deploy['id']}/files{$encoded}", file_get_contents($dist . $path), 'application/octet-stream', 600 + (int)ceil($size / 25000));
            if ($status >= 400) throw new RuntimeException("上传 {$path} 失败（HTTP {$status}）");
            $done++;
            if ($progress) $progress(['state' => 'uploading', 'files_total' => $total, 'files_done' => $done, 'current' => $path]);
        }
        if ($progress) $progress(['state' => 'waiting']);
    }
    // 构建等待放宽到 10 分钟：数千文件的站 Netlify 处理可能超 5 分钟，过早判死会误报失败（实际稍后成功）
    for ($i = 0; $i < 200; $i++) {
        sleep(3);
        [, $info] = netlify_request($token, 'GET', "https://api.netlify.com/api/v1/deploys/{$deploy['id']}");
        if (($info['state'] ?? '') === 'ready') return $info['ssl_url'] ?? ($info['url'] ?? '');
        if (($info['state'] ?? '') === 'error') throw new RuntimeException('Netlify 处理部署失败，请到控制台查看日志');
    }
    throw new RuntimeException('等待 Netlify 发布超时，请稍后到控制台查看结果');
}
