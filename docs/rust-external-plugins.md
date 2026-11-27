# 独立 Rust 插件

插件使用独立 Git 仓库和 Cargo 工程，通过 JSON 协议与 MUKE 通信，无须依赖、重编译或重启主程序。需要 MUKE v0.5.0 或更高版本。

## 安装

后台「插件 → 安装插件」选择来源：

- **GitHub**：公开仓库地址 `https://github.com/作者/仓库`，拉取默认分支。
- **本地**：MUKE 服务机器上的 Git 仓库绝对路径，包含未提交修改，忽略 `.git/` 和 `target/`。

多插件仓库先点击「扫描仓库」，选择插件子目录，再点击「编译并安装」，任务页可查看日志和取消。成功后刷新页面、填写配置并启用。更新使用「更新并重新编译」；失败或取消保留旧安装。

服务账号需能运行 Git、Rust/Cargo 和 C 编译工具，并访问依赖源或缓存。示例要求 Rust 1.91+；标准运行容器需自行补充编译工具。Cargo 缓存默认位于数据目录的 `runtime/plugin-cargo/`，可用 `CARGO_HOME` 指定。

**仅安装受信任的仓库**：构建脚本和插件拥有服务账号权限，独立进程不是权限沙箱。

## 仓库结构

复制 [示例](../examples/standalone-plugin) 到自己的仓库：

```text
Cargo.toml
Cargo.lock
plugin.json
src/main.rs
frontend/plugin.js    # 可选
frontend/plugin.css   # 可选
```

安装器执行 `cargo build --release --locked --bin <binary>`，安装程序文件和 `frontend/`。其他运行资源需嵌入程序；仓库不接受符号链接。

清单格式见 [plugin.json](../examples/standalone-plugin/plugin.json)：

- 必填 `protocol: 1`、`id`、`version`、`binary`、`name`、`methods`、`fields`。ID 与程序名以小写字母或数字开头，可含 `-`、`_`；版本为 `major.minor.patch`。
- `fields` 生成后台表单，支持 `text`、`number`、`select`；配置由主程序按插件隔离保存。
- `frontend` 声明脚本和样式；`page` 声明页面，使用时必须实现 `export`。ID 和路由不能与已有插件或核心冲突。

## 通信协议

每次调用启动一个进程，stdin 接收一行 JSON；stdout 每行输出一条 JSON 消息，日志写 stderr。最后输出一次 `result` 并正常退出。

```json
{"protocol":1,"method":"sync","params":{"all":false},"config":{"message":"你好"},"state":null}
```

`config` 是插件配置，`state` 是上次保存的私有 JSON 状态。`methods` 声明需要的方法，`check` 必须实现且无须声明。

| 方法 | 输入与返回 |
| --- | --- |
| `check` | `result` 返回与清单一致的 `protocol`、`id`、`version` |
| `sync` | `params` 为任务参数；返回 `posts`、可选 `state` 和业务 `result` |
| `data` | `params.query` 为查询参数，`params.admin` 为管理员标识；`result` 为页面数据 |
| `admin` | `params` 为管理输入；返回 `result` 和可选 `state` |
| `export` | `result` 返回公开静态数据 `{"items": ..., "meta": ...}` |

```json
{"type":"progress","phase":"同步中","completed":1,"total":10}
{"type":"result","result":{"message":"完成"},"state":{"cursor":"next"},"posts":[{"key":"item-1","content":"正文"}]}
```

批量任务可在业务 `result` 中返回 `"summary":{"total":3,"succeeded":2,"failed":1}`，任务页据此显示完成数量与部分失败；其他结果可用 `message` 提供简短说明。

失败返回 `{"type":"error","message":"错误原因"}`。单条消息最多 8 MB；同步最长 10 分钟，普通调用 30 秒，编译 30 分钟。取消或超时会终止进程组。

`posts` 仅供 `sync` 使用，每次最多 1000 条。必填 `key`、`content`；可选 `created_at`（RFC3339）、`link`（HTTP/HTTPS）、`meta`、`images`（最多 16 个远程图片 URL）。主程序负责图片下载、去重、删除记录与入库；失败前已提交的条目保留，同一 `key` 不会重复发布。

`state` 仅在 `sync`、`admin` 成功后保存。插件自行决定 `data`、`export` 公开哪些内容；当前协议不提供插件独立文件数据库。

## 公共模型服务

后台「模型」管理 Responses、Chat Completions、Anthropic Messages 配置。密钥由主程序保存，插件只传模型配置 ID。

清单声明 `"capabilities": ["llm.generate"]`。在 `sync` 中输出并刷新 stdout：

```json
{"type":"host_call","id":"analysis-1","method":"llm.generate","params":{"profile":1,"instructions":"分析要求","input":"待分析材料"}}
```

随后从 stdin 读取一行 `host_result`，按相同 `id` 关联：成功包含 `result: {"text": "…", "model": "…", "usage": {...}}`，失败包含 `error`。调用结束后继续返回插件最终结果。提示词最多 20 KB，材料最多 200 KB；页面读取和静态导出不能调用模型。

管理员可通过 `sync-plugin` 的 `params` 对象传入试运行等插件自定义任务参数。模型请求共用并发限制与任务取消，调用记录不保存密钥和提示词正文。

## 前端与备份

`frontend/` 发布到 `/plugins/<id>/`，通过 `MKBLOG.registerCard`、`registerPage`、`registerWidget` 扩展界面。公共模块从 `/assets/lib/` 导入。静态数据读取 `store.pluginsData[id]` 与 `store.pluginsMeta[id]`，参见示例。

无 JavaScript 页面使用导出结果的 `items.html`，静态动态卡片使用 `meta.html`；主程序会清理 HTML 后嵌入页面。

完整备份包含插件程序、前端、配置与状态，不包含源码和编译缓存。跨架构或系统迁移后需重新编译插件。
