# 架构

语言：[English](ARCHITECTURE.md) | [简体中文](ARCHITECTURE.zh-CN.md)

`humen-mcp` 是智能体和人类之间的 broker。

1. 智能体通过 MCP `tools/call` 调用 `ask_humen` 或异步工具。
2. 后端校验 MCP 载荷，创建 pending 请求信封，包含 `created_at`、`timeout_seconds` 和 `expires_at`。
3. 人类登录 Web UI，通过 WebSocket 和 REST 轮询 fallback 收到请求，UI 根据 `expires_at` 显示倒计时。
4. 人类回答选择、文本、图片审阅或步骤型任务。图片审阅可以引用远程 `image_url`，也可以嵌入 `image_base64`，Web UI 会渲染为 data URL。
5. 后端把人类回答返回给等待中的 MCP 调用，或写入异步回复邮箱供 `read_humen_replies` 读取。

如果请求先过期，后端会把它从 pending 请求中移除，放入回收站，发送 `request_expired` 事件，并返回 JSON-RPC `-32001` 错误，附带请求详情和重试建议。回收站保留时间由 `HUMEN_TRASH_RETENTION_SECONDS` 控制，清理由 `HUMEN_CLEANUP_INTERVAL_SECONDS` 控制。

当前版本把 pending 请求保存在内存里，方便部署和观察。用户记录、活跃周期和请求历史通过 `HUMEN_USERS_FILE` 与 SQLite 保存。后续如果迁移到更完整的请求、会话和审计持久化，不应改变 MCP 工具表面。

## 认证

后端支持：

- 管理员邮箱密码登录：`HUMEN_ADMIN_EMAIL` 和 `HUMEN_ADMIN_PASSWORD`。
- GitHub OAuth：配置 `HUMEN_GITHUB_CLIENT_ID` 和 `HUMEN_GITHUB_CLIENT_SECRET` 后启用。
- Passkey：登录后可注册，用于后续免密码登录。

普通用户推荐通过 GitHub OAuth 注册。邮箱密码登录只保留给管理员。

## MCP 流程

阻塞请求：

```mermaid
sequenceDiagram
    participant A as Agent
    participant S as humen-mcp
    participant H as Human UI
    A->>S: tools/call ask_humen
    S->>H: request_created
    H->>S: answer
    S->>A: MCP result
```

异步请求：

```mermaid
sequenceDiagram
    participant A as Agent
    participant S as humen-mcp
    participant H as Human UI
    A->>S: tools/call ask_humen_text_async
    S->>A: request_id
    S->>H: request_created
    H->>S: answer
    A->>S: read_humen_replies
    S->>A: replies
```

## 插件系统

插件是声明式 manifest，不执行任意动态库代码。服务端启动时从 `HUMEN_PLUGIN_DIR` 读取 `*.json` 和 `*.toml`，通过 `humen-mcp-sdk` 的类型校验：

- `request_templates`：可复用请求模板。
- `route_strategies`：面向人类目录的路由提示。
- `scoring_rules`：回答质量或风险判断的评分规则声明。
- `channels`：第三方通道声明，例如 webhook。

MCP 工具：

- `list_humen_plugins` 返回已加载插件及其能力。
- `create_humen_request_from_template` 用 `plugin-id/template-id` 创建异步人类请求。

模板文本支持 `{{variable}}` 替换。替换值来自工具调用的 `variables` 对象。

## 数据边界

- agent secret 绑定到人类账号，不是全局匿名 token。
- `#admin` 是保留标签，只能由服务端根据管理员身份派生。
- 人类目录遵守管理员配置的可见性策略。
- 人类回答是外部输入，调用方不应把它当作无需验证的执行指令。
