# MUKE（Rust 版）安装与升级

Rust 版是一个单二进制服务（内嵌数据库迁移与网络栈，静态链接 curl/OpenSSL），媒体处理依赖系统 `magick`（ImageMagick）。

## 安装（Linux x86_64 / aarch64）

GitHub Release 二进制在 Ubuntu 24.04 原生构建，需要 glibc 2.39 或更高版本。较旧系统可在本机编译，或从源码使用 `Dockerfile` 构建镜像。

先安装 ImageMagick：Arch 使用 `sudo pacman -S imagemagick`；Debian/Ubuntu 使用 `sudo apt install imagemagick`。ImageMagick 6 仅提供 `convert` 时，创建命令链接：

```sh
if ! command -v magick >/dev/null; then
  sudo ln -s "$(command -v convert)" /usr/local/bin/magick
fi
```

1. 从 [GitHub Releases](https://github.com/xxyangyoulin/muke/releases) 下载对应架构的 tar.gz 与 `SHA256SUMS`，校验、解压并自检：

   ```sh
   sha256sum --ignore-missing -c SHA256SUMS
   tar xzf muke-<版本>-<架构>.tar.gz
   cd muke-<版本>
   ./muke check          # 校验 magick、数据目录可写、前端与插件资源齐备
   ```

2. 复制包内 `muke.toml.example` 为 `muke.toml`，修改配置中的安装路径：`listen`、`data_dir`（数据库与上传图片）、`frontend`（包内 `frontend/` 目录）、`export_dir`（静态导出目录）。站点级设置（名称、评论、部署凭据等）登录后台修改，存数据库。

3. 启动并建号：

   ```sh
   ./muke --config muke.toml serve
   ```

   从 `setup_hosts` 列出的地址访问 `http://<listen>/admin` 创建管理员。HTTPS 反向代理部署时设置 `cookie_secure = true`，由代理把 `/api`（含 SSE）与 `/image` 转发到本服务，保持同源 Cookie。

## systemd

```sh
cp muke.service /etc/systemd/system/
# 按安装路径修改 WorkingDirectory / ExecStart / ReadWritePaths
systemctl daemon-reload && systemctl enable --now muke
journalctl -u muke -f
```

## Docker

在源码根目录运行：

```sh
docker compose up -d --build
```

默认仅向本机开放 8785 端口，数据与导出文件使用命名卷持久化。首次访问 `http://127.0.0.1:8785/admin` 创建管理员。远程访问可使用 SSH 隧道或配置 HTTPS 反向代理。

自定义配置可将 `muke.toml` 只读挂载到 `/opt/muke/muke.toml`。容器路径使用 `/opt/muke/data`、`/opt/muke/frontend`、`/opt/muke/dist`；绑定宿主目录时需确保容器用户有写权限。源码编译独立插件还需要安装 Rust 工具链、Git 及构建依赖，默认运行镜像未包含这些工具。

## 旧版本数据迁移

1. 停止旧服务写入，使用旧版本的完整备份功能保存最终备份。
2. 按配置示例创建 `muke.toml`，核对代理、时区和插件目录等本地设置。
3. 将完整备份恢复到新的空目录，避免与旧服务共用数据：

   ```sh
   ./muke restore final-backup.zip /opt/muke/data-new
   ```

4. 将 `data_dir` 指向恢复目录，运行 `./muke check`，启动后核对内容、图片、插件、任务和静态导出，再切换入口。

支持核心 schema_versions 版本为 4 的旧数据库，以及已使用数据库迁移表的 Rust 版本。更早的数据需先由对应旧版本升级；不支持直接导入任意旧库。

## 回退

回退使用切换前的备份与对应版本程序，不直接把升级后的数据库交回旧程序。切换后若已产生新数据，先在 Rust 后台导出 JSON 备份核对，再恢复旧快照会丢失这部分更新。

## 升级

替换二进制与 `frontend/`、`plugins/*/frontend/` 后重启即可；数据库迁移在启动时自动执行。升级前建议在后台开启定期备份，或停止服务后手动 `./muke backup <路径>`。

`muke export` 与 `muke backup` 是离线维护命令，与 `serve` 共用内容目录实例锁；服务运行时会拒绝执行，避免并行恢复任务或读取不一致的数据。在线导出使用管理后台，在线完整备份使用定期备份任务。`tasks`、`cancel` 等队列操作仍可在服务运行时使用。完整备份在数据库快照和媒体复制期间持有内容锁，应用内修改会等待备份结束。
