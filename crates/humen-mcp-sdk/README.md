# humen-mcp-sdk

Language: English | [简体中文](#简体中文)

Rust SDK types for `humen-mcp` community plugins.

Plugins are declared as JSON manifests that can extend:

- request templates
- route strategies
- scoring rules
- third-party channels

The server loads these manifests from `HUMEN_PLUGIN_DIR`. Plugin authors can use
this crate to generate and validate manifests before distributing them.

## 简体中文

`humen-mcp-sdk` 提供 `humen-mcp` 社区插件 manifest 的 Rust 类型。

插件以 JSON 或 TOML manifest 声明，可扩展：

- 请求模板
- 路由策略
- 评分规则
- 第三方通道

服务端从 `HUMEN_PLUGIN_DIR` 加载这些 manifest。插件作者可以使用这个 crate 生成和校验 manifest，再分发给社区使用。
