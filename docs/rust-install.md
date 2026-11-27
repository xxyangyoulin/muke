# MUKE（Rust 版）安装与升级

Rust 版是一个单二进制服务（内嵌数据库迁移与网络栈，静态链接 curl/OpenSSL），运行时只依赖系统 `magick`（ImageMagick）。PHP 版的完整功能对等说明见 [rust-prototype.md](rust-prototype.md)。

## 安装（Linux x86_64 / aarch64）

1. 解压发行包并自检：

   ```sh
   tar xzf muke-<版本>-<架构>.tar.gz
   cd muke-<版本>
   ./muke check          # 校验 magick、数据目录可写、前端与插件资源齐备
   ```

   缺少的依赖按提示安装：Arch `pacman -S imagemagick`；Debian/Ubuntu `apt install imagemagick`。

2. 准备配置（参考包内 `muke.toml.example`）：`listen`、`data_dir`（数据库与上传图片）、`frontend`（包内 `frontend/` 目录）、`export_dir`（静态导出目录）。站点级设置（名称、评论、部署凭据等）登录后台修改，存数据库。

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

替换二进制与 `frontend/`、`plugins/*/frontend/` 后重启即可；数据库迁移在启动时自动执行。升级前建议在后台开启定期备份或手动 `./muke backup <路径>`。
