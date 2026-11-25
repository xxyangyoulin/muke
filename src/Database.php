<?php
declare(strict_types=1);

function migrate_database(PDO $db): void {
    $db->exec('CREATE TABLE IF NOT EXISTS schema_versions (component TEXT PRIMARY KEY, version INTEGER NOT NULL)');
    $versions = $db->query('SELECT component, version FROM schema_versions')->fetchAll(PDO::FETCH_KEY_PAIR);
    $migrations = ['core' => ['version' => 4, 'run' => function (PDO $db): void {
$db->exec('CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, password TEXT NOT NULL)');
$db->exec('CREATE TABLE IF NOT EXISTS posts (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)');
$db->exec('CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)');
// 点赞：访客令牌去重（token 为浏览器生成的随机串，服务端不绑身份）
$db->exec('CREATE TABLE IF NOT EXISTS post_likes (post_id INTEGER NOT NULL, token TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY (post_id, token))');
$db->exec('CREATE TABLE IF NOT EXISTS plugin_tombstones (key TEXT NOT NULL, plugin TEXT NOT NULL, deleted_at TEXT NOT NULL, PRIMARY KEY (plugin, key))');
$tombstoneColumns = $db->query('PRAGMA table_info(plugin_tombstones)')->fetchAll(PDO::FETCH_ASSOC);
if (count(array_filter($tombstoneColumns, fn($column) => $column['pk'] > 0)) === 1) {
        $db->exec('ALTER TABLE plugin_tombstones RENAME TO plugin_tombstones_old');
        $db->exec('CREATE TABLE plugin_tombstones (key TEXT NOT NULL, plugin TEXT NOT NULL, deleted_at TEXT NOT NULL, PRIMARY KEY (plugin, key))');
        $db->exec('INSERT INTO plugin_tombstones SELECT key, plugin, deleted_at FROM plugin_tombstones_old');
        $db->exec('DROP TABLE plugin_tombstones_old');
}
// 原生评论：先发后审（approved 直显；同一 IP 哈希命中过垃圾的进 pending 隐蔽待审）
$db->exec('CREATE TABLE IF NOT EXISTS comments (id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, target_id INTEGER NOT NULL, author TEXT NOT NULL, email TEXT NOT NULL DEFAULT \'\', site TEXT NOT NULL DEFAULT \'\', content TEXT NOT NULL, created_at TEXT NOT NULL, status TEXT NOT NULL DEFAULT \'approved\', reply_to INTEGER, ip_hash TEXT NOT NULL DEFAULT \'\')');
$db->exec('CREATE INDEX IF NOT EXISTS idx_comments_target ON comments (kind, target_id, status, created_at)');
$postColumns = $db->query('PRAGMA table_info(posts)')->fetchAll(PDO::FETCH_COLUMN, 1);
if (!in_array('pinned', $postColumns, true)) $db->exec('ALTER TABLE posts ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0');
foreach (['images' => 'TEXT NOT NULL DEFAULT \'[]\'', 'meta' => 'TEXT NOT NULL DEFAULT \'\'', 'source' => 'TEXT NOT NULL DEFAULT \'\'', 'source_key' => 'TEXT NOT NULL DEFAULT \'\'', 'link' => 'TEXT NOT NULL DEFAULT \'\''] as $col => $ddl) {
    if (!in_array($col, $postColumns, true)) $db->exec("ALTER TABLE posts ADD COLUMN $col $ddl");
}
$db->exec('CREATE TABLE IF NOT EXISTS articles (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, content TEXT NOT NULL, format TEXT NOT NULL DEFAULT \'md\', images TEXT NOT NULL DEFAULT \'[]\', created_at TEXT NOT NULL, updated_at TEXT NOT NULL)');
$articleColumns = $db->query('PRAGMA table_info(articles)')->fetchAll(PDO::FETCH_COLUMN, 1);
if (!in_array('images', $articleColumns, true)) $db->exec("ALTER TABLE articles ADD COLUMN images TEXT NOT NULL DEFAULT '[]'");
if (!in_array('tags', $articleColumns, true)) $db->exec("ALTER TABLE articles ADD COLUMN tags TEXT NOT NULL DEFAULT ''");
if (!in_array('draft', $articleColumns, true)) $db->exec("ALTER TABLE articles ADD COLUMN draft INTEGER NOT NULL DEFAULT 0");
$db->exec('CREATE INDEX IF NOT EXISTS idx_posts_timeline ON posts (created_at DESC, id DESC)');
$db->exec('CREATE INDEX IF NOT EXISTS idx_posts_pinned_timeline ON posts (pinned DESC, created_at DESC, id DESC)');
$db->exec('CREATE INDEX IF NOT EXISTS idx_articles_timeline ON articles (draft, created_at DESC, id DESC)');
$db->exec('CREATE TABLE IF NOT EXISTS article_revisions (id INTEGER PRIMARY KEY AUTOINCREMENT, article_id INTEGER NOT NULL, title TEXT NOT NULL, content TEXT NOT NULL, format TEXT NOT NULL, tags TEXT NOT NULL, draft INTEGER NOT NULL, images TEXT NOT NULL, updated_at TEXT NOT NULL, saved_at TEXT NOT NULL)');
$db->exec('CREATE INDEX IF NOT EXISTS idx_article_revisions ON article_revisions (article_id, id DESC)');


    }]];
    foreach (plugin_list() as $id => $plugin) {
        if (isset($plugin['tables'])) $migrations['plugin:' . $id] = ['version' => $plugin['schema_version'] ?? 1, 'run' => $plugin['tables']];
    }
    foreach ($migrations as $component => $migration) {
        if ((int)($versions[$component] ?? 0) >= $migration['version']) continue;
        $db->beginTransaction();
        try {
            $db->exec('UPDATE schema_versions SET version = version WHERE 0');
            $current = $db->prepare('SELECT version FROM schema_versions WHERE component = ?');
            $current->execute([$component]);
            if ((int)$current->fetchColumn() >= $migration['version']) { $db->commit(); continue; }
            $migration['run']($db);
            $db->prepare('INSERT INTO schema_versions (component, version) VALUES (?, ?) ON CONFLICT(component) DO UPDATE SET version = excluded.version')->execute([$component, $migration['version']]);
            $db->commit();
        } catch (Throwable $e) { $db->rollBack(); throw $e; }
    }
}
