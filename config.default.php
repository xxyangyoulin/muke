<?php
// mkblog 默认配置。个人配置请复制为 config.php 修改，config.php 不会进入 Git。
// 大部分配置项可以用环境变量覆盖（见各项注释中的变量名）。
return [
    // ---- 品牌 ----
    // 站点名称（MKBLOG_SITE_NAME）
    'site_name' => '木刻',
    // 站点对外地址（含协议，如 https://example.com）：邮件通知/Akismet 等外部交互引用；留空则按请求推导
    'site_url' => '',
    // 侧栏品牌副标题（MKBLOG_SITE_SUBTITLE）
    'site_subtitle' => 'MUKE JOURNAL',
    // 站点标语，出现在浏览器标题和导出页面标题（MKBLOG_TAGLINE）
    'site_tagline' => '记录与风景',
    // 搜索引擎摘要（MKBLOG_DESC）
    'site_description' => '记录生活，分享灵感，收藏每一片风景。',
    // 界面语言与日期格式的 locale，如 zh-CN / en-US（MKBLOG_LOCALE）
    'locale' => 'zh-CN',
    // PHP date() 格式：文章阅读页日期 / 列表页日期（MKBLOG_DATE_FORMAT / MKBLOG_DATE_FORMAT_SHORT）
    'date_format' => 'Y年n月j日',
    'date_format_short' => 'n月j日',

    // ---- 内容限额（前后端共用） ----
    // 单条动态最长字数（MKBLOG_POST_MAX_CHARS）
    'post_max_chars' => 500,
    // 每条动态最多图片数（MKBLOG_POST_MAX_IMAGES）
    'post_max_images' => 4,
    // 单张图片最大体积（MB）（MKBLOG_UPLOAD_MAX_MB）
    'upload_max_mb' => 15,
    // 单篇文章正文最大体积（字节）（MKBLOG_ARTICLE_MAX_BYTES）
    'article_max_bytes' => 2097152,

    // ---- 图片处理 ----
    // 列表缩略图长边像素与质量（MKBLOG_THUMB_SIZE / MKBLOG_THUMB_QUALITY）
    'thumb_size' => 720,
    'thumb_quality' => 78,
    // 头像裁剪尺寸（像素，正方形）（MKBLOG_AVATAR_SIZE）
    'avatar_size' => 400,

    // ---- 首次建号 ----
    // 允许创建首个管理员的来源 IP 白名单；加 '*' 允许任意来源（公网部署时请尽快完成建号并移除）
    // （MKBLOG_SETUP_HOSTS，逗号分隔）
    'setup_hosts' => ['127.0.0.1', '::1'],

    // ---- 前端展示 ----
    // 阅读时长估算：每分钟字数（MKBLOG_READING_CPM）
    'reading_chars_per_minute' => 500,

    // ---- 数据与网络 ----
    // 数据目录：SQLite 数据库、动态图片、缩略图缓存（MKBLOG_DATA_DIR）
    'data_dir' => __DIR__ . '/var',
    'timezone' => 'Asia/Shanghai',
    // 部署等外部请求使用的 HTTP 代理，例如 http://127.0.0.1:7890（MKBLOG_HTTP_PROXY）
    'http_proxy' => '',
    // 发布新动态后自动部署到 Netlify：'1' 开启 / '0' 关闭（后台「静态发布」页签可改）
    'auto_deploy' => '0',
    // 自动部署防抖（分钟）：新动态顺延计时，静默这么多分钟后才执行（后台可改，1–1440）
    'auto_deploy_debounce' => 10,

    // ---- 首次运行的默认资料（可在后台修改） ----
    'profile_bio' => '记录日常的微小瞬间，收集值得停留的风景。',
    'profile_handle' => 'muke',
];
