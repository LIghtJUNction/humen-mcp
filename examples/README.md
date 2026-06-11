# MCP Examples

Language: [English](README.md) | [简体中文](README.zh-CN.md)

Use these payloads with `POST /mcp` and an agent secret header:

```bash
curl -fsS http://127.0.0.1:8787/mcp \
  -H 'content-type: application/json' \
  -H "x-humen-agent-secret: ${HUMEN_AGENT_SECRET}" \
  --data @examples/mcp-tools-list.json
```

Async human requests prefer MCP notifications. Clients that support
Streamable HTTP SSE can open a notification stream:

```bash
curl -N http://127.0.0.1:8787/mcp \
  -H 'accept: text/event-stream' \
  -H "x-humen-agent-secret: ${HUMEN_AGENT_SECRET}"
```

When a human reply is available, the stream emits
`notifications/humen/reply_available`; call `read_humen_replies` for the full
answer. If the stream is unavailable, keep using `read_humen_replies` as the
polling fallback.

The examples mirror the tool schemas in `src/mcp.rs`.

| File | Tool or method |
| --- | --- |
| `mcp-initialize.json` | `initialize` |
| `mcp-tools-list.json` | `tools/list` |
| `mcp-approve.json` | `approve` |
| `mcp-judge.json` | `judge` |
| `mcp-feedback.json` | `feedback` |
| `mcp-ask-choice.json` | `ask_humen` |
| `mcp-ask-image-base64.json` | `ask_humen` image review |
| `mcp-ask-text-async.json` | `ask_humen_text_async` |
| `mcp-ask-judgment-async.json` | `ask_humen_judgment_async` |
| `mcp-read-replies.json` | `read_humen_replies` |
| `mcp-list-nodes.json` | `list_humen_nodes` |
| `mcp-search-network.json` | `search_humen_network` |
| `mcp-ask-network-async.json` | `ask_humen_network_async` |
| `mcp-read-network-ledger.json` | `read_humen_network_ledger` |
| `mcp-list-plugins.json` | `list_humen_plugins` |
| `mcp-create-from-template.json` | `create_humen_request_from_template` |
| `mcp-create-task.json` | `create_humen_task` |
| `mcp-list-tasks.json` | `list_humen_tasks` |
| `mcp-leave-memo.json` | `leave_humen_memo` |
| `mcp-list-agent-inbox.json` | `list_agent_inbox` |
| `mcp-request-human-friend.json` | `request_human_friend` |
| `mcp-accept-human-friend.json` | `accept_human_friend` |
| `mcp-list-online-humens.json` | `list_online_humens` |
| `mcp-search-profiles.json` | `search_humen_profiles` |
| `mcp-list-tags.json` | `list_humen_tags` |
| `mcp-rate-humen.json` | `rate_humen` |
| `mcp-report-humen.json` | `report_humen` |

`community-release-plugin.json` is a plugin manifest example, not an MCP
JSON-RPC payload. Copy it into a directory and set `HUMEN_PLUGIN_DIR` to that
directory before starting the server.
