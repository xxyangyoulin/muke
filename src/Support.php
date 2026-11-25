<?php
declare(strict_types=1);

function h(string $text): string {
    return htmlspecialchars($text, ENT_QUOTES, 'UTF-8');
}

/** 代码指纹（app + src + 插件声明的 mtime 汇总）：常驻进程（PM2 worker / 自动部署 watcher）
 *  在启动时缓存了全部 PHP 代码与插件注册表，磁盘上新增或修改插件对它们不可见；
 *  指纹变化时自行退出，交给 PM2 autorestart 或 worker 拉起新代码的进程。 */
function code_fingerprint(): string {
    clearstatcache();
    $files = array_merge([dirname(__DIR__) . '/app.php'], glob(dirname(__DIR__) . '/src/*.php') ?: [], glob(dirname(__DIR__) . '/src/*/*.php') ?: [], glob(dirname(__DIR__) . '/plugins/*/plugin.php') ?: []);
    return md5(implode('|', array_map(fn($f) => $f . '=' . @filemtime($f), $files)));
}
