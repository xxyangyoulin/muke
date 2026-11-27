# 木刻 MUKE

Rust + SQLite 的个人动态、文章与收藏站。单二进制后端，独立 HTML/CSS/JS 前端，支持动态运行和导出到 Netlify。

![项目预览](docs/preview.png)

## 功能

- 动态、图片、置顶、Markdown 文章、修订历史与搜索
- RSS、豆瓣、微博、壁纸和桌面更新日志插件
- 原生评论与点赞，或接入 giscus / Waline / Twikoo
- 静态导出、Netlify 部署、统一后台任务与取消操作
- 完整备份恢复、定期备份

当前发布版：**0.5.0**，详见 [变更记录](CHANGELOG.md)。

## 快速开始

从 [Releases](https://github.com/xxyangyoulin/muke/releases) 下载对应架构安装包及 `SHA256SUMS`。需要 Linux（glibc 2.39+，如 Ubuntu 24.04）和 ImageMagick。

```sh
sha256sum --ignore-missing -c SHA256SUMS
tar xzf muke-0.5.0-x86_64.tar.gz  # ARM64 使用 aarch64 包
cd muke-0.5.0
./muke check
./muke serve
```

打开 `http://127.0.0.1:8785/admin` 创建管理员。配置、systemd、Docker、升级与 PHP 数据迁移见 [安装指南](docs/rust-install.md)。

## 独立插件

后台安装：填写公开 GitHub 仓库地址，或服务机器上的本地仓库路径，扫描并选择插件后即可编译、安装和更新。需要 Git、Rust/Cargo 和 C 编译工具。

后台「模型」统一配置 OpenAI / 兼容接口和 Anthropic，供插件调用。[独立插件仓库](https://github.com/xxyangyoulin/muke-plugins) 提供 GitHub 项目关注插件。

插件作者只维护自己的 Cargo 工程和前端资源，无须依赖或重编译 MUKE。参见 [开发指南](docs/rust-external-plugins.md) 和 [最小示例](examples/standalone-plugin)。现有内置插件仍随主程序编译。

## 开发

```sh
cargo build --locked
cargo test --locked
cargo run -- serve
```

后端位于 `src/`，核心前端位于 `frontend/`，内置插件位于 `plugins/<id>/`。接口边界见 [插件架构](docs/rust-plugin-boundaries.md)。旧 PHP 代码保留用于迁移与回归验证。

MIT，见 [LICENSE](LICENSE)。
