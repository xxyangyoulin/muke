<?php
declare(strict_types=1);

function dms_read_snapshot(string $directory, string $packages = '/var/lib/pacman/local'): array {
    $directory = rtrim($directory, '/');
    if ($directory === '') throw new RuntimeException('请先配置 DMS 配置目录');
    if (!is_dir($directory . '/plugins') || !is_readable($directory . '/plugins')) throw new RuntimeException('DMS 插件目录不存在或不可读，请检查路径与权限');
    $plugins = [];
    foreach (glob($directory . '/plugins/*/plugin.json') as $file) {
        if (!is_readable($file)) throw new RuntimeException('DMS 插件清单不可读：' . basename(dirname($file)));
        $data = json_decode((string)file_get_contents($file), true);
        if (!is_array($data) || !is_string($data['id'] ?? null) || $data['id'] === '' || !is_string($data['name'] ?? null) || $data['name'] === '' || (isset($data['version']) && !is_string($data['version']))) {
            throw new RuntimeException('DMS 插件清单格式无效：' . basename(dirname($file)));
        }
        $plugins[$data['id']] = ['name' => $data['name'], 'version' => $data['version'] ?? ''];
    }
    ksort($plugins);
    $version = null;
    // Arch 安装记录可直接读取；其他安装方式没有可靠版本来源时不发布版本事件。
    foreach (glob(rtrim($packages, '/') . '/{dux-shell,dank-material-shell,dms}*/desc', GLOB_BRACE) as $file) {
        if (!is_readable($file)) continue;
        $desc = (string)file_get_contents($file);
        if (preg_match('/^%NAME%\R((?:dux-shell|dank-material-shell|dms)(?:-git|-bin)?)\R/m', $desc, $name)
            && preg_match('/^%VERSION%\R([^\r\n]+)\R/m', $desc, $value)) {
            $version = ['package' => $name[1], 'version' => $value[1]];
            break;
        }
    }
    return ['directory' => realpath($directory), 'plugins' => $plugins, 'kernel' => php_uname('r'), 'dms' => $version];
}

$nextSnapshot = null;
return [
    'id' => 'dms',
    'name' => '桌面更新日志',
    'description' => '直接检测 DMS 插件安装与版本变化、运行内核变化，以及 Arch 的 DMS 软件包版本变化。首次同步只建立基线。',
    'label_color' => '#6750a4',
    'fields' => [
        ['key' => 'config_dir', 'label' => 'DMS 配置目录', 'type' => 'text', 'required' => true, 'max' => 1000, 'pattern' => '#^/#', 'hint' => '包含 plugins 子目录的绝对路径，例如 /home/用户名/.config/DankMaterialShell'],
        ['key' => 'sync_minutes', 'label' => '同步间隔（分钟）', 'type' => 'number', 'min' => 1, 'max' => 1440, 'default' => 1],
    ],
    'tables' => function (PDO $db): void {
        $db->exec("DELETE FROM settings WHERE key IN ('plugin:dms:events_db', 'plugin:dms:cursor', 'plugin:dms:music_day')");
    },
    'sync' => function (PDO $db, array $config) use (&$nextSnapshot): array {
        $nextSnapshot = null;
        $current = dms_read_snapshot((string)($config['config_dir'] ?? ''));
        $old = json_decode((string)(settings($db)['plugin:dms:snapshot'] ?? ''), true);
        $revision = (int)($old['revision'] ?? 0);
        $nextSnapshot = $current + ['revision' => $revision + 1];
        if (!$old || ($old['directory'] ?? '') !== $current['directory']) return [];
        $changes = [];
        $installed = [];
        $updated = [];
        foreach ($current['plugins'] as $id => $plugin) {
            $previous = $old['plugins'][$id] ?? null;
            if ($previous === null) $installed[] = $plugin['name'];
            elseif ($previous['version'] !== '' && $plugin['version'] !== '' && $previous['version'] !== $plugin['version']) {
                $updated[] = $plugin['name'] . '（' . $previous['version'] . ' → ' . $plugin['version'] . '）';
            }
        }
        if ($installed) $changes['installed'] = '安装了新的 DMS 插件：' . implode('、', $installed);
        if ($updated) $changes['updated'] = 'DMS 插件版本更新：' . implode('、', $updated);
        if ($old['kernel'] !== $current['kernel']) $changes['kernel'] = 'Linux 运行内核已切换：' . $old['kernel'] . ' → ' . $current['kernel'];
        if ($old['dms'] !== null && $current['dms'] !== null && $old['dms'] !== $current['dms']) {
            $changes['dms'] = 'DMS 软件包版本更新：' . $old['dms']['version'] . ' → ' . $current['dms']['version'];
        }
        $items = [];
        foreach ($changes as $kind => $content) {
            $items[] = ['key' => 'snapshot:' . hash('sha256', json_encode([$current['directory'], $revision, $kind, $content])), 'content' => $content];
        }
        return $items;
    },
    'after_sync' => function (PDO $db, array $items) use (&$nextSnapshot): void {
        // 入库成功后才推进基线；重试继续使用同一事件键，由核心去重。
        if ($nextSnapshot !== null) save_setting($db, 'plugin:dms:snapshot', json_encode($nextSnapshot, JSON_UNESCAPED_UNICODE));
    },
];
