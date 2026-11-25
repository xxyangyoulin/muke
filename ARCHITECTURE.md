# 架构说明

## 目录总览

```
app.php                 内核装配：require src/ 全部模块 + 引导（会话、数据库、核心表迁移、插件表迁移）
src/
  Config.php            config()：config.default.php + config.php + 环境变量 三层合并
  Database.php          核心与插件版本迁移（schema_versions）
  Backup.php            数据库快照、资源 ZIP 备份及空目录恢复
  PublicData.php        运行时与静态导出共用的公开配置、插件描述
  Support.php           h()（HTML 转义）
  Settings.php          settings()/opt()（后台设置 > 配置默认）、post_meta()、日期格式化
  Http.php              json_response()、http_fetch()（含代理与重试，可传自定义头）
  Images.php            上传/缩略图/图片响应（up: 核心处理，其余协议转发插件）/远程图入库
  Articles.php          文章域：Markdown 渲染（Parsedown）、标签、图片引用解析（post_images）
  Comments.php          评论域：匿名评论、先发后审、蜜罐/限流/垃圾 IP 联动待审、Akismet/Gravatar
  Mail.php              极简 SMTP 客户端（评论通知；未配置静默跳过）
  Export.php            静态导出管线（prepare→profile→plugins→posts→articles→json→bake→feeds→cleanup）
  Plugins.php           插件系统：发现、配置、同步入库（幂等 + 墓碑）、调度、页面/引用解析通用机制
  Netlify.php           Netlify 摘要式部署
  Api.php               API 分发器（GET/POST 路由表）
  Api/Feed.php          state / posts / articles / plugin-data（GET）
  Api/Content.php       动态与文章的增删改、预览、图片上传（删动态带插件 after_post_delete 钩子）
  Api/Plugins.php       插件列表 / 保存 / 移除（可选清史）/ 同步（含全部历史）/ plugin-data
  Api/System.php        建号登录、资料、密码、头像、站点参数、导出、部署、备份恢复
export.php              静态导出编排：数据 → 媒体复制 → 插件导出 → 烘焙 HTML（含 RSS/sitemap）
public/index.php        路由（页面 / api / image / 插件资源）+ HTML 骨架 + CSP + 插件导航
public/assets/          前端 SPA（原生 ESM，零构建）
  registry.js           MKBLOG 注册表：registerCard / registerPage / registerWidget（经典脚本，先于模块加载）
  app.js                装配：SPA 路由（goto/renderPage，含插件页）、事件委托、init()
  lib/store.js          共享可变状态（唯一数据源，视图经 import 读写；插件数据放 pluginsData）
  lib/api.js            fetch 封装（api POST / apiGet GET）
  lib/lifecycle.js      页面切换时清理监听器/观察器，阻止过期响应覆盖当前页
  lib/ui.js             图标/转义/toast/日期/动态渲染等纯函数与组件
  components/           lightbox.js（自研灯箱）、composer.js（发布器）、comments.js（评论）
  views/                feed / articles / editor / admin 四个视图（feed 侧栏挂载插件小组件）
plugins/<id>/           插件（完全自包含：plugin.php + plugin.js + plugin.css）
lib/Parsedown.php       Markdown 渲染（vendor）
bin/                    CLI：check.php 自检、export.php 导出、deploy-netlify.php 部署、backup.php / restore.php 完整备份恢复
tests/                  冒烟套件：run.sh（lint + 沙箱 + API + 浏览器 + 导出）
```

## 请求流

```
浏览器 → public/index.php
  ├─ /api            → api($db) → 路由表 → src/Api/*.php 处理器 → json_response
  ├─ /image          → src/Images.php（avatar/upload 核心处理；?file= 转发插件 image_endpoint）
  ├─ /plugin-asset/* → plugins/<id>/plugin.(js|css)
  ├─ /embed/comments  → Waline/Twikoo 同源嵌入页（独立宽松 CSP，仅放行 unpkg/jsdelivr）
  └─ 页面             → HTML 骨架（插件导航/标签由插件声明）→ app.js 按 pathname 渲染视图（插件页由 plugin.js 注册）
```

## 关键机制

- **配置三层**：环境变量 > `config.php` > 核心与插件默认值；插件用 `defaults` / `env` 声明专属配置。后台「站点参数」与插件设置写入 settings 表后经 `opt()` 优先于文件配置。
- **动态插件**：`plugins/<id>/plugin.php` 声明字段与 `sync()`；条目以 `source_key` 幂等入库，用户删除立墓碑（`plugin_tombstones`）永不再同步；`meta.card` 选择卡片样式，前端由插件的 `plugin.js` 经 `MKBLOG.registerCard` 渲染，无 JS 时用 `render_post` 烘焙。插件可声明 `timeline_field`：对应配置关闭时其动态整体退出首页时间线与站点 RSS（如 RSS 订阅默认只在自有标签页展示）；`page.tab` 让插件页面进首页顶部标签（`state.plugins` 提供 route/title）。
- **插件契约（完全自包含）**：除卡片外，插件还可声明——
  - `tables` + `schema_version`：自带建表/数据迁移，仅版本增加时执行，由 `src/Database.php` 记录版本并在事务中迁移
  - `page`：公开页面（核心放行路由、渲染导航、静态导出）；`plugin.js` 经 `MKBLOG.registerPage` 提供前端渲染
  - `data`：公开数据端点（GET `?action=plugin-data&id=<id>`）
  - `resolve_ref`：自定义图片引用协议（如 `wall:<id>`）→ 动态图片描述符
  - `image_endpoint`：`/image?file=` 的文件服务（插件自带越界防护）
  - `export`：静态导出（复制媒体、生成引用映射与无 JS 页面）
  - `after_post_delete`：用户删除动态后的插件侧清理
  - `post_images_full`：插件动态的图片不套用普通动态的单图紧凑样式
  - `MKBLOG.registerWidget`：在首页侧栏挂载小组件（如壁纸精选）
- **插件事件总线**：核心在内容生命周期触发 `plugin_fire`（post/article 的 saved/deleted、comment 的 created/moderated）；插件以 `hooks` 键订阅，订阅者异常隔离不影响主流程。
- **评论双形态**：动态部署=原生（先发后审）；静态部署=第三方注入（giscus/custom 外链 iframe + frame-src 自动例外；Waline/Twikoo 同源嵌入页 + 分区 CSP）+ 原生已审评论烘焙只读。
- **壁纸生命周期**（业务与配置声明均在 `plugins/wallpapers/`）：扫描登记；多选上传按方向归档并调用公共插件入库接口合并为一条动态；源文件删除只移除动态中的对应图片，最后一张删除时清动态；用户删除动态走按插件隔离的墓碑。停用后不扫描，导出仅保留历史动态引用的图片。
- **静态导出**：与运行时同一套数据（posts + articles + 各插件导出），烘焙无 JS 可读的 HTML，RSS/sitemap 一并生成；插件页与插件资源（plugin-assets/）随导出复制。

## 读取与发布边界

- 动态与文章 API 按 `(created_at, id)` 游标分页，默认 30 条、最多 100 条，并支持服务端搜索；文章支持精确标签筛选，草稿通过管理员专用的 `draft=1` 查询分页。首页混合时间线、文章列表与后台按需加载；静态导出保持完整数据。文章列表仅渲染当前页摘要，详情及编辑另取正文。
- 排序索引由核心第 2 版迁移建立；动态、文章列表及详情的评论计数只查询当前内容 ID。公开标签云聚合全部已发布文章的标签字段，不读取正文或草稿。
- 壁纸管理删除会同时移除所有引用动态中的对应图片；多图动态保留其余图片，最后一张删除后才清理整条动态及评论、点赞。文件先临时改名，数据库失败时恢复；成功后更新前端缓存，不需要刷新首页。
- 壁纸 `data` 与导出只读取已登记记录；目录扫描只在同步或上传流程发生。目录离线视为同步失败，不当作空目录清理。`state` 仍会调度后台同步，并非彻底无副作用的读取入口。
- 静态导出先在临时目录全量构建并检查产物，再以目录重命名替换 `dist`；构建失败保留上一版。替换有短暂间隙，不是跨目录原子交换；缩略图也会重建。
- 数据库提交后才删除旧媒体、派发事务内的插件保存事件。JSON 导入遇到可捕获的失败会回滚图片替换；数据库与文件系统仍不能保证进程崩溃时的跨资源原子性。
- 完整 ZIP 备份与后台内容 JSON 分开；前者覆盖插件声明的外部资源，恢复只接受空目录。插件是受信任代码，协议/路由冲突检查不等于权限沙箱。

## 开发入口

- 新增 API 动作：在 `src/Api/<域>.php` 加 `api_<action>()` 处理器，在 `src/Api.php` 对应路由表注册一行。
- 新增插件：见 [docs/plugins.md](docs/plugins.md)（完整契约能力菜单 + 最小示例；可只做动态卡片，也可带页面/数据端点/自定义引用）。
- 改完跑 `bash tests/run.sh`（ lint + 冒烟全绿再提交）。
