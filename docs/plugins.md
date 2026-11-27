# 插件开发

第三方插件在自己的仓库中开发，可在一个仓库内维护多个插件。后台填写 GitHub 仓库地址或本地路径，即可扫描、编译和安装。

- [独立插件协议与开发示例](rust-external-plugins.md)
- [核心与内置插件边界](rust-plugin-boundaries.md)

内置插件的后端和前端分别位于 `plugins/<id>/backend/` 与 `frontend/`。业务代码留在插件目录，核心仅提供通用服务。
