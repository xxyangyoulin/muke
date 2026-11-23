# 插件开发指南

一个插件 = 一个目录，**全部代码自包含**：

```
plugins/<id>/
  plugin.php   # 元数据 + 配置字段 + sync() 拉取 + 契约能力（见下）
  plugin.js    # 前端：卡片 / 页面 / 小组件注册（可选，按需自动加载）
  plugin.css   # 样式（可选，按需自动加载）
```

## 最小插件

在 `plugins/<id>/plugin.php` 返回一个数组（必填 `id`，与目录名一致；`sync` 仅同步类插件需要）：

```php
<?php
return [
    'id' => 'weibo',
    'name' => '微博动态',
    'description' => '同步最近的微博到日常动态。',
    'fields' => [   // 后台自动生成配置表单
        ['key' => 'user', 'label' => '用户名', 'type' => 'text', 'required' => true],
        ['key' => 'sync_minutes', 'label' => '同步间隔（分钟）', 'type' => 'number', 'min' => 5, 'max' => 1440, 'default' => 30],
    ],
    'sync' => function (PDO $db, array $config, array $options = []): array {
        // 拉取内容，返回条目数组（每次可全量返回，入库时按 key 去重）
        return [[
            'key' => 'weibo:123456',            // 幂等键，必填且稳定
            'content' => '一条微博内容……',
            'created_at' => '2026-11-21T10:00:00+08:00',
            'link' => 'https://weibo.com/...',   // 可选：动态上的「原文」链接（仅 http/https）
            'images' => ['https://...jpg'],      // 可选：远程图自动下载入库；也可用 up: 或插件自定义引用
        ]],
    },
];
```

约定：`key` 在各插件内幂等去重；用户删除过的 `(plugin, key)` 会进墓碑不再同步，不影响其他插件。`images` 完整保留，插件自行约束外部来源的数量；普通动态发布仍使用站点的图片数量限制。`sync` 抛错会记录到卡片的「上次失败」并跳过本轮。

插件主动发布动态也使用 `plugin_sync_items($db, $id, $items)`，统一处理引用、去重、墓碑与 `post.saved` 事件，不直接写入核心 `posts` 表。在事务中必须传入第四参数 `$events = []` 接收事件，成功提交后再遍历调用 `plugin_fire('post.saved', $event)`；回滚则丢弃。无事务时立即触发。

配置可声明 `defaults`（配置键 → 默认值）与 `env`（环境变量名 → 配置键）。核心合并插件默认值后应用 `config.php` 和环境变量；配置键须避免与核心或其他插件冲突。插件文件加载时仅返回声明，不调用配置或访问数据库。

核心助手可直接使用：`http_fetch($url, $timeout, $headers, $allowPrivate)`（自动走 `http_proxy`；默认阻断内网地址，管理员配置的内网源显式传 `true`）、`fetch_remote_image($url, $minSize, $headers)`、`post_meta()`。

## 契约能力菜单（全部可选，按需声明）

| 声明 | 能力 |
| --- | --- |
| 条目 `meta.card` + `plugin.js` 的 `MKBLOG.registerCard(样式名, fn)` + `render_post($post)` | 自定义动态卡片（无 JS 时用 `render_post` 烘焙静态版）|
| `tables` 闭包 + `schema_version` | 自带建表 / 数据迁移；版本默认 1，仅版本增加时在事务中执行，需支持从旧表升级 |
| `page` 声明 + `plugin.js` 的 `MKBLOG.registerPage(路由, {render})` | 拥有整页路由；`page.description` 提供静态页 SEO 摘要，核心不识别具体插件路由 |
| `data` 闭包 | 公开数据端点 `GET /api?action=plugin-data&id=<id>` |
| `ref_protocols` + `resolve_ref` 闭包 | 声明独占协议名列表，再解析图片引用（如 `['wall']` 对应 `wall:<id>`）；协议与页面路由冲突会拒绝加载 |
| `backup_paths` 闭包 | 返回「目录配置键 → 本地目录」；完整 ZIP 备份包含这些资源，恢复时重定位对应配置 |
| `image_endpoint` 闭包 | `GET /image?file=` 的文件服务（越界防护自备）|
| `export($db, $dest, &$files, $enabled)` 闭包 | 静态导出：复制媒体、生成引用映射与无 JS 页面；停用但有历史动态时 `$enabled=false`，只能导出现有内容所需媒体，不扫描、同步或修改业务数据 |
| `admin_op` 闭包 | 管理操作端点 `POST /api?action=plugin-admin`（仅管理员，op 自解释）|
| `on_config_change` 闭包 | 配置保存后回调（可重烙已有数据，使开关即时生效）|
| `after_sync` / `after_post_delete` 闭包 | 同步后 / 用户删除动态后的插件侧处理 |
| `plugin.js` 的 `MKBLOG.registerWidget(id, fn)` | 首页侧栏小组件 |
| `hooks` => ['post.saved' => fn, …] | 事件总线：post.saved / post.deleted / article.saved / article.deleted / comment.created / comment.moderated |
| `post_images_full` => true | 插件动态的图片保持原始尺寸（不套单图紧凑样式）|
| `supports_history` => true | 卡片出现「同步历史」按钮（`sync` 收到 `options['all']`）|

停用后不运行同步、事件订阅和管理操作，不提供插件页面及数据端点；历史动态的图片引用解析与图片服务保留。没有历史动态的停用插件不执行导出。`tables` 为数据库初始化，`on_config_change` 为管理员配置操作，不属于运行时事件。移除插件会清除配置，外部目录等依赖需另行备份，不能视为仅停用。

参考实现：`plugins/wallpapers/` 演示页面、上传、媒体引用和静态导出；`plugins/douban/`、`plugins/weibo/` 演示动态卡片与内容同步。插件是受信任的 PHP/JS 代码，不是权限隔离沙箱。

架构细节（数据流、请求流、导出管线）见 [ARCHITECTURE.md](../ARCHITECTURE.md)。
