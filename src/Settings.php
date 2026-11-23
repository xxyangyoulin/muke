<?php
declare(strict_types=1);

function settings(PDO $db): array {
    return $db->query('SELECT key, value FROM settings')->fetchAll(PDO::FETCH_KEY_PAIR);
}

function save_setting(PDO $db, string $key, string $value): void {
    $stmt = $db->prepare('INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value');
    $stmt->execute([$key, $value]);
}

function opt(string $key): mixed {
    global $db;
    $values = settings($db);
    return array_key_exists($key, $values) ? $values[$key] : config($key);
}

function post_meta(array $post): array {
    $m = $post['meta'] ?? '';
    return is_array($m) ? $m : (json_decode((string)$m, true) ?: []);
}

function date_short_label(string $atom): string {
    $ts = strtotime($atom) ?: time();
    $fmt = (int)date('Y', $ts) === (int)date('Y') ? (string)opt('date_format_short') : (string)opt('date_format');
    return date($fmt, $ts);
}
