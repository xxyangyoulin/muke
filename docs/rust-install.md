# MUKE（Rust 版）安装与升级

Rust 版是一个单二进制服务（内嵌数据库迁移与网络栈，静态链接 curl/OpenSSL），媒体处理依赖系统 `magick`（ImageMagick）。PHP 版的功能对等说明见仓库中的 [Rust 实现记录](https://github.com/xxyangyoulin/muke/blob/master/docs/rust-prototype.md)。

## 安装（Linux x86_64 / aarch64）

GitHub Release 二进制在 Ubuntu 24.04 原生构建，需要 glibc 2.39 或更高版本。较旧系统可在本机编译，或从源码使用 `Dockerfile.rust` 构建镜像。

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

```sh
docker build -f Dockerfile.rust -t muke .
docker run -d -p 8785:8785 \
  -v $PWD/data:/opt/muke/data \
  -v $PWD/muke.toml:/opt/muke/muke.toml:ro \
  -v $PWD/dist:/opt/muke/dist \
  muke
```

容器内已包含 ImageMagick；数据与导出目录必须挂载出来持久化。

## 从 PHP 版迁移

1. 停止 PHP 写入：`pm2 stop mkblog-sync`，并暂停 Web 入口（Nginx 指向维护页或直接停 PHP-FPM）。
2. 在旧安装目录做最终完整备份：`php bin/backup.php /安全路径/final-backup.zip`。
3. 转换配置：`php bin/rust-migrate-config.php config.php muke.toml`，按需补充 `listen` / `frontend` / `export_dir`。
4. 恢复数据到新目录（Rust 与 PHP 不能同时写同一份数据）：

   ```sh
   mkdir /opt/muke/data-empty
   ./muke --data-dir /opt/muke/data restore final-backup.zip /opt/muke/data-empty
   mv /opt/muke/data-empty /opt/muke/data
   ```

5. `./muke check` → 启动 → 后台核对内容、任务页、静态导出与部署；确认后切换 Nginx 入口。
6. 壁纸目录等插件资源：恢复库的设置表已指向恢复目录内的副本，可在后台改回原路径。

## 回退

回退使用切换前的备份与对应 PHP 代码，不直接把升级后的数据库交回旧程序。切换后若已产生新数据，先在 Rust 后台导出 JSON 备份核对，再恢复旧快照会丢失这部分更新。

## 升级（Rust → Rust）

替换二进制与 `frontend/`、`plugins/*/frontend/` 后重启即可；数据库迁移在启动时自动执行。升级前建议在后台开启定期备份，或停止服务后手动 `./muke backup <路径>`。

`muke export` 与 `muke backup` 是离线维护命令，与 `serve` 共用内容目录实例锁；服务运行时会拒绝执行，避免并行恢复任务或读取不一致的数据。在线导出使用管理后台，在线完整备份使用定期备份任务。`tasks`、`cancel` 等队列操作仍可在服务运行时使用。完整备份在数据库快照和媒体复制期间持有内容锁，应用内修改会等待备份结束。
