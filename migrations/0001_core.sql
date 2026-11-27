CREATE TABLE IF NOT EXISTS schema_versions (component TEXT PRIMARY KEY, version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, password TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS posts (
 id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL,
 created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
 pinned INTEGER NOT NULL DEFAULT 0, images TEXT NOT NULL DEFAULT '[]',
 meta TEXT NOT NULL DEFAULT '', source TEXT NOT NULL DEFAULT '',
 source_key TEXT NOT NULL DEFAULT '', link TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS articles (
 id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL, content TEXT NOT NULL,
 format TEXT NOT NULL DEFAULT 'md', images TEXT NOT NULL DEFAULT '[]',
 tags TEXT NOT NULL DEFAULT '', draft INTEGER NOT NULL DEFAULT 0,
 created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS article_revisions (
 id INTEGER PRIMARY KEY AUTOINCREMENT, article_id INTEGER NOT NULL, title TEXT NOT NULL,
 content TEXT NOT NULL, format TEXT NOT NULL, tags TEXT NOT NULL, draft INTEGER NOT NULL,
 images TEXT NOT NULL, updated_at TEXT NOT NULL, saved_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS post_likes (post_id INTEGER NOT NULL, token TEXT NOT NULL, created_at TEXT NOT NULL, PRIMARY KEY(post_id,token));
CREATE TABLE IF NOT EXISTS plugin_tombstones (key TEXT NOT NULL, plugin TEXT NOT NULL, deleted_at TEXT NOT NULL, PRIMARY KEY(plugin,key));
CREATE TABLE IF NOT EXISTS comments (
 id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL, target_id INTEGER NOT NULL,
 author TEXT NOT NULL, email TEXT NOT NULL DEFAULT '', site TEXT NOT NULL DEFAULT '',
 content TEXT NOT NULL, created_at TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'approved',
 reply_to INTEGER, ip_hash TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_posts_timeline ON posts(created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS idx_posts_pinned_timeline ON posts(pinned DESC,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS idx_articles_timeline ON articles(draft,created_at DESC,id DESC);
CREATE INDEX IF NOT EXISTS idx_article_revisions ON article_revisions(article_id,id DESC);
CREATE INDEX IF NOT EXISTS idx_comments_target ON comments(kind,target_id,status,created_at);
INSERT INTO schema_versions(component,version) VALUES('core',4) ON CONFLICT(component) DO NOTHING;
