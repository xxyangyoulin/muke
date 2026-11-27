# MUKE 独立插件示例

把本目录内容复制到自己的 Git 仓库，修改 `plugin.json` 中的 ID、名称、版本和 `src/main.rs` 中的业务处理即可。此 Cargo 工程不依赖 MUKE 主仓库。

本地验证：

```sh
cargo build --release --locked
printf '%s\n' '{"protocol":1,"method":"check","params":{},"config":{},"state":null}' | target/release/muke-plugin-hello
```

在 MUKE 后台的「插件 → 安装插件」填写自己的公开 GitHub 仓库地址，或选择「本地仓库」并填写服务机器上的仓库绝对路径。安装完成后刷新页面、启用插件，然后同步；`/hello` 页面同时支持动态站与静态导出。

开发协议见 [MUKE 独立插件指南](https://github.com/xxyangyoulin/muke/blob/master/docs/rust-external-plugins.md)。更改 ID 时，同时修改 `check` 响应、前端数据键和页面路由；修改版本时同步更新 Cargo.toml、Cargo.lock 与 plugin.json。
