# 项目结构

MUKE 使用 Rust 单进程服务、SQLite 数据库和独立的原生 JavaScript 前端。

- `src/`：HTTP API、认证、内容、图片、静态导出、备份、模型服务和持久任务队列。
- `migrations/`：启动时自动执行的数据库迁移。
- `frontend/`：核心页面、组件和插件加载器，通过 HTTP API 与后端交互。
- `plugins/<id>/backend/`、`frontend/`：内置插件的业务逻辑与界面，后端编译进主程序。
- `examples/standalone-plugin/`：第三方独立进程插件示例。
- `tests/`：单元、API、浏览器及固定数据回归测试；入口为 `bash tests/run.sh`。

后台任务统一管理进度、并发、取消与恢复。插件通过通用接口使用任务、媒体、模型服务和内容导入，核心不包含具体插件业务。

插件接口与一致性约束见[插件边界](docs/rust-plugin-boundaries.md)，部署见[安装指南](docs/rust-install.md)。
