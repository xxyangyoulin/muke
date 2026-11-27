# 木刻 MUKE

当前版本：**0.3.0**。版本号以 [VERSION](VERSION) 为准，变更记录见 [CHANGELOG.md](CHANGELOG.md)。

PHP + SQLite 的个人动态 / 文章 / 壁纸收藏站。零框架、零构建：前端原生 HTML/CSS/JS，后端函数式 PHP，唯一第三方库是 [Parsedown](https://parsedown.org)（Markdown 渲染）。

支持两种发布形态：**动态部署**（自有服务器 / 内网，自带评论与点赞）与**静态部署**（一键导出纯 HTML 部署到 Netlify，评论接第三方系统）。

![项目预览](docs/preview.png)

## 功能一览

- **动态时间线**：X 风格布局，短文字 + 最多 4 图，可编辑删除，服务端点赞计数
- **动态置顶**：管理员可置顶 / 取消置顶，支持多条；首页、分页和静态导出保持一致
- **文章随笔**：Markdown / 富文本双模式、标签、草稿、独立详情页
- **文章修订历史**：编辑页查看历史版本及差异，一键恢复标题、正文、标签和发布状态；保存变更和恢复前自动保留原版本及图片，重复保存相同内容不产生新版本。仅管理员可访问，JSON / 完整备份包含历史；删除文章会同时删除修订记录并清理不再引用的图片。启用前的修改无法追溯，静态导出不包含历史记录。
- **文章编辑保护**：未保存修改时，返回后台、站内跳转及浏览器前进后退会提醒；刷新 / 关闭页面由浏览器提示。输入后约 0.7 秒自动暂存标题、正文、标签和编辑模式，再次进入同一篇文章（新文章点击“写文章”）可选择恢复或丢弃，恢复不会自动发布。保存到服务器成功后清除暂存，失败则保留。暂存仅在当前站点、当前浏览器中保存，不跨设备同步、不包含在服务器备份中；清理浏览器数据会删除暂存，共享电脑上请留意内容隐私。浏览器可能限制关闭提醒，强制结束进程也可能丢失最后不足一秒的输入。
- **站内搜索**：首页搜索支持全部 / 动态 / 文章筛选及关键词高亮，动态站和静态导出站均可使用
- **壁纸瀑布流**：本地目录自动同步，筛选 / 搜索 / 4K 标记 / 原图下载，新壁纸自动发动态
- **评论**：匿名评论 + 审核 + 蜜罐限流 + 可选 Akismet 与邮件通知；静态站接 giscus / Waline / Twikoo
- **插件系统**：内容源可插拔（内置豆瓣、微博、壁纸三个参考实现），支持自定义卡片 / 页面 / 事件订阅
- **静态导出 + 一键部署**：无 JS 可读的纯静态站，Netlify 摘要式增量上传
- **后台任务与备份**：集中查看同步 / 部署 / 备份状态，可配置定期完整备份及保留份数

## 快速开始

依赖：PHP 8.1+（pdo_sqlite / sqlite3 / mbstring / curl / fileinfo / zip）、ImageMagick。

从 [GitHub Releases](https://github.com/xxyangyoulin/muke/releases) 下载 `muke-<版本>.zip` 和 `SHA256SUMS`，运行 `sha256sum -c SHA256SUMS` 校验，解压后进入 `muke-<版本>/`。这是可运行的项目安装包，不是静态站点包，也不包含 PHP、ImageMagick、用户数据或个人配置。不要用它覆盖自己的 `var/` 和 `config.php`。

```sh
# 1. 环境自检
php bin/check.php

# 2. 本地临时预览（正式运行使用下文的 Nginx + PHP-FPM）
php -S 127.0.0.1:8765 -d upload_max_filesize=16M -d post_max_size=68M -t public public/index.php

# 3. 打开后台建号（默认仅允许本机来源）
#    http://127.0.0.1:8765/admin
```

Docker：`docker compose up -d`（挂载 `var/` 与壁纸目录）。

生产环境使用 PHP-FPM 配合 Nginx/Apache，将文档根指向 `public/`，不存在的路由转发给 `public/index.php`；PHP 内置服务器用于本地运行。Docker 首次建号可通过 SSH 转发或临时设置精确的 `MKBLOG_SETUP_HOSTS` 来源地址，建号后撤销放行；不要长期设置为 `*`。

### Nginx Web + PM2 后台任务

Web 使用 Nginx + PHP-FPM 多进程处理请求；PM2 只守护后台同步，不再启动 PHP 内置 Web 服务器。配置 Nginx 时只允许入口 `public/index.php` 交给 FPM 执行，禁止上传目录中的 PHP 执行；Nginx 请求体上限设为 `68m`，PHP 的 `upload_max_filesize` / `post_max_size` 分别设为 `16M` / `68M`。

以站点运行用户执行：

```sh
pm2 start ecosystem.config.cjs
pm2 save
pm2 startup
# 首次配置时，执行 pm2 startup 输出的 sudo 命令以启用开机恢复
pm2 status
pm2 logs mkblog-sync --lines 50
```

`MKBLOG_NAME` 可修改进程名前缀（默认 `mkblog`），`PHP_BIN` 可指定 PHP CLI（默认 `/usr/bin/php`）。PHP-FPM 与后台进程需使用相同配置和文件访问权限；不要将 `PHP_BIN` 指向 `php-fpm`。升级代码后执行 `pm2 restart mkblog-sync`；已有旧版 PM2 Web 进程时，先确认其名称，再执行 `pm2 delete mkblog`，避免与 Nginx 抢占端口。

当前本机站点使用 `http://127.0.0.1:8765`，Nginx 配置为 `/etc/nginx/mkblog.conf`，FPM 专用池为 `/etc/php/php-fpm.d/mkblog.conf`，Web 服务名为 `nginx-mkblog`。修改本机 Nginx 配置后执行 `sudo nginx -t -c /etc/nginx/mkblog.conf && sudo systemctl reload nginx-mkblog`。这些是本机配置路径，其他服务器应按实际环境配置，不由 PM2 创建。

本机 `pm2-yyl.service` 已启用，使用 oneshot 开机恢复方式（现有 PM2 未提供标准 PID 文件）；任务重启由 PM2 管理。日常使用 `pm2 restart mkblog-sync`，不要为重启同步而重启整个 PM2 服务，以免影响同一用户下的其他应用。

## 版本与项目安装包发布

采用 `major.minor.patch` 版本号。先修改 `VERSION` 和 `CHANGELOG.md`，提交本次发布所需的全部代码，再推送对应标签：

```sh
git tag v0.3.0
git push origin v0.3.0
```

标签必须与该提交的 `VERSION` 一致。GitHub Actions 会先运行完整测试，再生成项目安装 ZIP 和 `SHA256SUMS`，上传至对应 GitHub Release；不执行静态站点导出。使用仓库自带的 `GITHUB_TOKEN`，不需要配置个人令牌；仓库须允许 GitHub Actions 运行及工作流申请 `contents: write` 权限。

本地可执行 `bash scripts/package.sh v0.3.0`（仅在 Git 源码仓库中使用），产物在 `build/release/`。打包只读取 **HEAD 中已提交的文件**，不包含尚未提交的修改；私有配置、数据库、上传图片、壁纸原图、缓存和 `dist/` 均不打包。新的版本请创建新的标签，不移动已发布标签；更新版本号时同步更新本页版本与命令示例。

升级前先做完整备份。将新版本解压至新目录，接回自己的配置与数据目录，运行环境自检后再切换服务；数据库迁移会在首次启动执行，回退旧代码可能还需要恢复升级前的数据备份。

## 配置

两层：**后台「站点设置」界面修改即生效**（运行参数：标语、壁纸目录、限额、评论、缩略图等），`config.php`（个人覆盖，不入库不入 Git）承载基础设施项。完整默认值见 `config.default.php`，常用环境变量：

| 环境变量 | 说明 |
| --- | --- |
| `MKBLOG_DATA_DIR` | 数据目录（SQLite、上传、缓存），默认 `var/` |
| `MKBLOG_WALLPAPER_DIR` | 壁纸源目录（后台亦可改） |
| `MKBLOG_SITE_URL` | 对外地址，canonical / RSS / 邮件引用（如 `https://eyee.top`） |
| `MKBLOG_SETUP_HOSTS` | 允许首次建号的来源 IP（默认仅本机，`*` 为任意） |
| `MKBLOG_HTTP_PROXY` | 出站请求代理（部署 / 拉取远程内容） |

## 发布内容

- **动态 / 文章**：后台直接发布；文章详情页每篇一个 URL，可被收录
- **动态置顶**：登录后点击动态右上角的置顶图标（插件动态也支持）。多条置顶按发布时间倒序排列在普通动态和文章之前；取消后回归时间顺序。置顶变更会触发已开启的自动部署，RSS 仍按发布时间排序
- **壁纸**：把图片放进壁纸目录即可，自动登记；管理后台可删除
- **静态站**：后台「静态发布」→ 生成 / 下载 zip / 一键部署 Netlify（首次填令牌 + 站点 ID）；可选「发布新动态后自动部署」（默认关，防抖 10 分钟，新动态顺延计时）。静态站无任何管理入口，账号密码数据库等全留在本地

内容更新流程：本地发布 → 重新导出部署（一键部署会自动先导出）。

## 插件

`plugins/` 目录下的插件提供内容源与页面，后台「动态插件」页签管理（开关 / 配置 / 同步 / 清空 / 移除）。内置豆瓣、微博（RSSHub）、壁纸和[桌面更新日志（DMS）](plugins/dms/README.md)，分别覆盖内容同步、自定义卡片、整页路由、数据端点、上传、图片服务、本地桌面版本检测与静态导出。壁纸专属默认配置与环境变量声明位于其 `plugin.php`。

开发自己的插件见 **[docs/plugins.md](docs/plugins.md)**，架构与数据流见 **[ARCHITECTURE.md](ARCHITECTURE.md)**。

前后端分离的 Rust 版已完成并接管生产：单二进制服务（`src/`）+ 独立前端（`frontend/`）+ 插件前后端同目录（`plugins/<id>/`），统一后台任务（图片/插件同步/导出/部署/备份）、静态导出与 Netlify 部署、完整备份恢复。安装、迁移与回退见 [Rust 安装指南](docs/rust-install.md)，实现进度与验证记录见 [Rust 原型进度](docs/rust-prototype.md)。PHP 版保留可回退。

删除语义：手动删除插件同步的动态会留墓碑（不再同步回来）；壁纸源文件删除则清动态、文件回归可重发。

启用插件时会校验已有配置与本次输入合并后的完整配置；缺少必填项时拒绝启用，不会部分保存。停用不要求补齐配置。

### 独立定时同步

`pm2 start ecosystem.config.cjs` 只启动独立的 `mkblog-sync` 后台进程，网站由 Nginx + PHP-FPM 提供；`docker compose up -d --build` 则同时启动容器网站和 `sync` 服务。同步进程每轮检查后等待 30 秒，各插件仍遵循自己的同步间隔，设置为 0 的壁纸插件只允许手动同步。无需访客访问页面，进程退出由 PM2 / Docker 自动重启；PM2 开机恢复需要同时完成 `pm2 save` 和 `pm2 startup` 配置。

其他运行方式可执行 `php bin/sync.php --watch` 并交由进程管理器管理，或使用 cron 每分钟运行一次：

```cron
* * * * * /usr/bin/php /absolute/path/mkblog/bin/sync.php
```

同步进程应与网站使用相同的用户、配置、数据目录和壁纸目录。页面触发与独立同步共用文件锁，不会同时执行同步。PM2 日志位于 `var/sync-out.log`、`var/sync-error.log`；Docker 可用 `docker compose logs sync` 查看。

开启自动部署且配置 Netlify 后，壁纸上传、删除、目录扫描发现变更，以及插件同步新增动态都会启动防抖计时；没有内容变化的同步不会反复延后部署。手动和自动部署共用全程文件锁，冲突任务不会覆盖正在执行的部署状态。

自动部署结束时只清理本次处理的待办；部署期间产生的新更新会保留，并按新的防抖到期时间继续部署。

后台同步进程还会检查遗留的待部署记录：如果重启导致部署等待进程消失，会重新启动它并沿用原定到期时间，无需访问网页。部署等待进程仍按需启动，不是单独的 PM2 常驻项；已执行失败并清除待办的部署不会自动重试。

### 任务状态与定期备份

后台「任务」页每 5 秒刷新：后台常驻进程是否运行、最近检查阶段、各插件上次尝试 / 下次同步 / 最近错误、部署状态与排期，以及自动备份结果。已到期时间表示等待后台检查，不保证在该秒执行；长时间同步或备份会推迟后续检查。使用 cron 而非常驻进程时，页面会显示未检测到常驻进程。

定期备份默认关闭，在「任务 → 定期备份设置」启用。默认间隔 24 小时、保留 7 份，可设置为 1–720 小时和 1–30 份。首次启用后由后台下一轮检查执行，此后按上次开始时间计算间隔；失败会显示错误，并等下个间隔再尝试，不会每 30 秒重试。

备份使用现有完整 ZIP 格式，包含数据库、上传图片及插件声明的资源（含壁纸原图），保存到数据目录下的 `backups/`，不位于 Web 根目录。文件名以 `muke-auto-` 开头；只有新备份成功后才清理超出保留份数的自动 ZIP，手动备份不参与轮换。关闭定期备份不会删除已有文件。恢复仍使用下文的 `bin/restore.php`，不会自动覆盖当前数据。

请预留至少一份完整备份的额外空间；备份期间为保证一致性会占用数据库写事务，修改内容可能等待。外部程序应避免同时改写资源。备份目标不能位于被备份的资源目录内；同盘备份不能防磁盘损坏，重要数据还应另行离机保存。升级后台代码后运行 `pm2 restart mkblog-sync` 使常驻进程加载新功能。

### giscus 评论

公开 GitHub 仓库开启 Discussions，并安装 [giscus App](https://github.com/apps/giscus) 授权该仓库。在 [giscus 配置页](https://giscus.app/zh-CN)选择仓库及 Announcements 类型分类，将 `data-repo`、`data-repo-id`、`data-category-id` 填入后台「评论 → 评论设置」，选择 giscus 后保存。无需在网站配置 GitHub 令牌。

文章与动态分别按 `article:ID`、`post:ID` 关联讨论，动态站和静态导出共用相同映射；评论需 GitHub 登录，在仓库 Discussions 管理，原生评论审核、邮件与反垃圾设置不作用于 giscus。静态站需重新导出和部署才能应用配置。已有原生评论不会自动迁移到 GitHub。giscus 模式下，动态点赞按钮展开反应区，登录 GitHub 后选择 ❤️ 即可在动态站和静态站共用点赞；展开组件后会更新卡片的爱心数量和当前用户状态。历史本地点赞不迁移。每次静态导出会通过 giscus 公开接口查询评论总数（含全部分页的回复）和 ❤️ 数量，写入首页、文章及 RSS 订阅卡片数据，无需额外服务器或令牌。打开评论区前显示最近一次导出的统计，下一次部署刷新；计数不代表访客个人点赞状态。接口失败会中止本次导出并保留上一版产物，避免错误归零。

## 备份与安全

- 核心数据在 `var/`，壁纸原图可以位于外部目录。完整备份：`php bin/backup.php /absolute/path/backup.zip`，包含数据库快照、上传图片、头像/封面和插件声明的资源目录。
- 恢复到已创建的空目录：`php bin/restore.php backup.zip /absolute/path/restored-data`，再将 `MKBLOG_DATA_DIR` 指向该目录。恢复会重定位插件资源目录；不会覆盖当前数据。代码、`config.php`、环境变量与部署凭据需另外保管。备份期间暂停外部程序改写资源文件。
- 后台 JSON 导入/导出是内容备份（含评论与上传图片），不包含账号、壁纸原图及插件业务表，不能代替完整备份。
- 可手动执行 `php tests/restore-rehearsal.php /absolute/path/empty-directory` 演练：只读快照现有数据库，在隔离副本上完成备份、恢复、逐表及文件哈希校验和迁移。需预留资源总量约三倍的空间，暂停外部资源写入；结束后自动清理生成的副本与压缩包，不启动同步、部署或邮件任务，也不替代真实登录验证。
- 安全机制：CSRF 全覆盖、登录限速（5 次失败锁 15 分钟）、评论蜜罐 + 频控、Markdown 原始 HTML 转义、出站请求 SSRF 防护、上传 MIME 白名单

## 许可

MIT，见 [LICENSE](LICENSE)。
