# Architecture

Language: [English](ARCHITECTURE.md) | [简体中文](ARCHITECTURE.zh-CN.md)

`humen-mcp` is a broker between agents and humans.

1. Agent calls MCP `tools/call` for `ask_humen`.
2. Backend validates the MCP payload and creates a pending request envelope with `created_at`, `timeout_seconds`, and `expires_at`.
3. Human logs into the web UI and receives the request over WebSocket plus REST polling fallback, including a live countdown from `expires_at`.
4. Human answers a simple choice, text, image review, or step-following task. Image review requests can reference a remote `image_url` or embed `image_base64` data that the web UI renders as a data URL.
5. Backend resolves the waiting MCP call with the human answer.

If the envelope expires first, the backend removes it from pending requests, stores it in the in-memory trash bin, sends `request_expired`, and returns JSON-RPC error code `-32001` with request details and a retry suggestion. Trash is retained for `HUMEN_TRASH_RETENTION_SECONDS` and cleaned on `HUMEN_CLEANUP_INTERVAL_SECONDS`.

The first version intentionally keeps pending requests and trash in memory so the full loop is easy to deploy and inspect. User records and WebSocket active periods are persisted in `HUMEN_USERS_FILE`; the next persistence step should add SQLite/Postgres for requests, trash, sessions, and audit events without changing the MCP surface.

## Auth

The backend supports:

- Email/password login through `HUMEN_ADMIN_EMAIL` and `HUMEN_ADMIN_PASSWORD`.
- GitHub OAuth endpoints when `HUMEN_GITHUB_CLIENT_ID` and `HUMEN_GITHUB_CLIENT_SECRET` are set.

Production deployment should replace the single configured admin with database-backed users before inviting multiple humans.
