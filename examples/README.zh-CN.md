# MCP 示例

语言：[English](README.md) | [简体中文](README.zh-CN.md)

这些 payload 可配合 `POST /mcp` 和 agent secret header 使用：

```bash
curl -fsS http://127.0.0.1:8787/mcp \
  -H 'content-type: application/json' \
  -H "x-humen-agent-secret: ${HUMEN_AGENT_SECRET}" \
  --data @examples/mcp-tools-list.json
```

示例文件与 `src/mcp.rs` 中的工具 schema 保持一致。

| 文件 | 工具或方法 |
| --- | --- |
| `mcp-initialize.json` | `initialize` |
| `mcp-tools-list.json` | `tools/list` |
| `mcp-approve.json` | `approve` |
| `mcp-judge.json` | `judge` |
| `mcp-feedback.json` | `feedback` |
| `mcp-ask-choice.json` | `ask_humen` |
| `mcp-ask-image-base64.json` | `ask_humen` 图片审阅 |
| `mcp-ask-text-async.json` | `ask_humen_text_async` |
| `mcp-ask-judgment-async.json` | `ask_humen_judgment_async` |
| `mcp-read-replies.json` | `read_humen_replies` |
| `mcp-list-plugins.json` | `list_humen_plugins` |
| `mcp-create-from-template.json` | `create_humen_request_from_template` |
| `mcp-create-task.json` | `create_humen_task` |
| `mcp-list-tasks.json` | `list_humen_tasks` |
| `mcp-list-agent-inbox.json` | `list_agent_inbox` |
| `mcp-request-human-friend.json` | `request_human_friend` |
| `mcp-accept-human-friend.json` | `accept_human_friend` |
| `mcp-list-online-humens.json` | `list_online_humens` |
| `mcp-search-profiles.json` | `search_humen_profiles` |
| `mcp-list-tags.json` | `list_humen_tags` |
| `mcp-rate-humen.json` | `rate_humen` |
| `mcp-report-humen.json` | `report_humen` |

`community-release-plugin.json` 是插件 manifest 示例，不是 MCP JSON-RPC payload。把它复制到一个目录，并在启动服务前把 `HUMEN_PLUGIN_DIR` 指向该目录。
