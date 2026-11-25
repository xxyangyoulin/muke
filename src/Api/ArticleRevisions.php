<?php
declare(strict_types=1);

function api_article_revisions(PDO $db, bool $admin): never {
    if (!$admin) json_response(['error' => '请先登录管理后台'], 401);
    $id = (int)($_GET['id'] ?? 0);
    $article = $db->prepare('SELECT id FROM articles WHERE id = ?');
    $article->execute([$id]);
    if (!$article->fetchColumn()) json_response(['error' => '文章已不存在'], 404);
    if (isset($_GET['revision'])) {
        $stmt = $db->prepare('SELECT * FROM article_revisions WHERE article_id = ? AND id = ?');
        $stmt->execute([$id, (int)$_GET['revision']]);
        $revision = $stmt->fetch(PDO::FETCH_ASSOC);
        if (!$revision) json_response(['error' => '历史版本不存在'], 404);
        json_response(['revision' => $revision]);
    }
    $cursor = (int)($_GET['cursor'] ?? 0);
    $stmt = $db->prepare('SELECT id, title, draft, updated_at, saved_at FROM article_revisions WHERE article_id = ?' . ($cursor ? ' AND id < ?' : '') . ' ORDER BY id DESC LIMIT 21');
    $stmt->execute($cursor ? [$id, $cursor] : [$id]);
    $revisions = $stmt->fetchAll(PDO::FETCH_ASSOC);
    $more = count($revisions) > 20;
    if ($more) array_pop($revisions);
    json_response(['revisions' => $revisions, 'next_cursor' => $more ? end($revisions)['id'] : null]);
}

function api_restore_article_revision(PDO $db, array $input, string $action, bool $admin): never {
    $id = (int)($input['id'] ?? 0);
    $db->beginTransaction();
    try {
        $db->exec('UPDATE articles SET id = id WHERE 0');
        $stmt = $db->prepare('SELECT * FROM articles WHERE id = ?');
        $stmt->execute([$id]);
        $article = $stmt->fetch(PDO::FETCH_ASSOC);
        $stmt = $db->prepare('SELECT * FROM article_revisions WHERE article_id = ? AND id = ?');
        $stmt->execute([$id, (int)($input['revision'] ?? 0)]);
        $revision = $stmt->fetch(PDO::FETCH_ASSOC);
        if (!$article || !$revision) { $db->rollBack(); json_response(['error' => '文章或历史版本已不存在'], 404); }
        update_article_with_revision($db, $article, $revision);
        $stmt = $db->prepare('SELECT * FROM articles WHERE id = ?');
        $stmt->execute([$id]);
        $restored = $stmt->fetch(PDO::FETCH_ASSOC);
        $db->commit();
    } catch (Throwable $e) { $db->rollBack(); throw $e; }
    plugin_fire('article.saved', ['id' => $id, 'title' => $restored['title'], 'draft' => (int)$restored['draft']]);
    json_response(['ok' => true, 'article' => $restored]);
}
