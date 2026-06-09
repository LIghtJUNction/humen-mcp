# MCP Examples

Use these payloads with `POST /mcp` and an agent secret header:

```bash
curl -fsS http://127.0.0.1:8787/mcp \
  -H 'content-type: application/json' \
  -H "x-humen-agent-secret: ${HUMEN_AGENT_SECRET}" \
  --data @examples/mcp-tools-list.json
```

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
| `mcp-create-task.json` | `create_humen_task` |
| `mcp-list-tasks.json` | `list_humen_tasks` |
| `mcp-list-online-humens.json` | `list_online_humens` |
| `mcp-search-profiles.json` | `search_humen_profiles` |
| `mcp-list-tags.json` | `list_humen_tags` |
| `mcp-rate-humen.json` | `rate_humen` |
| `mcp-report-humen.json` | `report_humen` |
