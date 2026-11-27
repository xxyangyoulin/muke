-- giscus 计数缓存：导出/部署按可配置间隔复用，避免每次全量查询
CREATE TABLE IF NOT EXISTS giscus_counts (
    term TEXT PRIMARY KEY,
    counts TEXT NOT NULL,
    fetched_at TEXT NOT NULL
);
