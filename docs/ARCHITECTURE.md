# Architecture

`humen-mcp` is a broker between agents and humans.

1. Agent calls MCP `tools/call` for `ask_humen`.
2. Backend validates the MCP payload and creates a pending request.
3. Human logs into the web UI and receives the request over WebSocket plus REST polling fallback.
4. Human answers a simple choice, text, image review, or step-following task.
5. Backend resolves the waiting MCP call with the human answer.

The first version intentionally keeps state in memory so the full loop is easy to deploy and inspect. The next persistence step should add SQLite/Postgres for users, sessions, requests, and audit events without changing the MCP surface.

## Auth

The backend supports:

- Email/password login through `HUMEN_ADMIN_EMAIL` and `HUMEN_ADMIN_PASSWORD`.
- GitHub OAuth endpoints when `HUMEN_GITHUB_CLIENT_ID` and `HUMEN_GITHUB_CLIENT_SECRET` are set.

Production deployment should replace the single configured admin with database-backed users before inviting multiple humans.

