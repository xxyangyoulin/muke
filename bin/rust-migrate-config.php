<?php
declare(strict_types=1);
// 生成 Rust 版配置：php bin/rust-migrate-config.php [config.php] [muke.toml]
// 读取 PHP 配置数组，输出 Rust 服务可用的 TOML。默认读取 ./config.php，输出 ./muke.toml。
// 结构字段（data_dir/timezone/setup_hosts/http_proxy）直接映射；其余键进入 [settings]。
// 迁移步骤见 docs/rust-rebuild-plan.md「数据迁移与回退」。

$configFile = $argv[1] ?? __DIR__ . '/../config.php';
$outputFile = $argv[2] ?? __DIR__ . '/../muke.toml';
$config = require $configFile;
if (!is_array($config)) { fwrite(STDERR, "配置文件没有返回数组\n"); exit(1); }

// Rust Config 结构字段；值保持原样（字符串化）
$struct = [];
foreach (['data_dir', 'timezone', 'http_proxy'] as $key) {
    if (isset($config[$key]) && is_string($config[$key]) && $config[$key] !== '') {
        $struct[$key] = $config[$key];
    }
}
if (isset($config['setup_hosts']) && is_array($config['setup_hosts'])) {
    $struct['setup_hosts'] = $config['setup_hosts'];
}

$settings = [];
foreach ($config as $key => $value) {
    if (in_array($key, ['data_dir', 'timezone', 'http_proxy', 'setup_hosts'], true)) continue;
    if (is_array($value)) { fwrite(STDERR, "设置 {$key} 是数组，Rust 版不支持，请手工处理\n"); exit(1); }
    $settings[$key] = is_bool($value) ? ($value ? '1' : '0') : (string)$value;
}

$escape = fn(string $s) => '"' . str_replace(['\\', '"'], ['\\\\', '\\"'], $s) . '"';
$lines = ["# 由 bin/rust-migrate-config.php 从 " . basename($configFile) . " 生成"];
foreach ($struct as $key => $value) {
    if ($key === 'setup_hosts') {
        $lines[] = 'setup_hosts = [' . implode(', ', array_map($escape, $value)) . ']';
    } else {
        $lines[] = "{$key} = " . $escape((string)$value);
    }
}
if ($settings) {
    $lines[] = '';
    $lines[] = '[settings]';
    foreach ($settings as $key => $value) $lines[] = "{$key} = " . $escape($value);
}
file_put_contents($outputFile, implode("\n", $lines) . "\n");
echo "已生成 {$outputFile}（" . count($settings) . " 项设置）\n";
echo "下一步：target/debug/muke --config " . $outputFile . " tasks 检查启动；正式迁移前先做完整备份。\n";
