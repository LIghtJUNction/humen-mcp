# humen-mcp

Human-in-the-loop MCP server. Agents call a tool named `ask_humen`; a logged-in human sees the request in the web UI, performs a simple task, and sends the answer back to the waiting agent call.

## Shape

- Rust backend: HTTP API, WebSocket updates, MCP JSON-RPC endpoint at `/mcp`.
- Bun web UI: `humen-mcp-webui` is kept as a git submodule.
- Deployment target: reverse proxy under `https://your-domain.example/mcp`, with systemd on Arch Linux.
- Packaging targets: `humen-mcp-git` and `humen-mcp-bin` AUR packages.
- Presence: the web UI shows the live count of connected human workbench sessions.
- Envelopes: every `ask_humen` call creates a pending request with `created_at`, `timeout_seconds`, and `expires_at`; expired requests move to trash.
- Auth: admin-only email/password login; GitHub OAuth is manually enabled by configuring client credentials, and non-admin users register/login only through GitHub.

## Local Run

```bash
cp env.example .env
cargo run
```

Build the web UI inside the submodule:

```bash
cd humen-mcp-webui
bun install
bun run build
```

Then restart `cargo run`; the backend serves the UI from `HUMEN_WEB_DIST`.

## MCP Endpoint

Configure an MCP client to send streamable HTTP / JSON-RPC requests to:

```text
https://your-domain.example/mcp
```

`POST /mcp` is the MCP JSON-RPC endpoint. `GET /mcp` is intentionally not the web UI; the human workbench is served at `/mcp/`.

Implemented methods:

- `initialize`
- `notifications/initialized`
- `tools/list`
- `tools/call` with tool `ask_humen`

Example JSON-RPC payloads live in `examples/`.

`ask_humen` accepts:

```json
{
  "kind": "choice|text|image_review|steps",
  "title": "Short task title",
  "prompt": "What the human should do",
  "choices": ["A", "B"],
  "image_url": "https://...",
  "steps": ["Open the site", "Read the SMS code"],
  "timeout_seconds": 60
}
```

If `timeout_seconds` is omitted, the backend uses 60 seconds. Expired requests return JSON-RPC error code `-32001` with request data and are available from `GET /api/trash` until cleanup.

## HTTP API

- `GET /api/requests`
- `GET /api/trash`
- `POST /api/trash/clear`
- `GET /api/ws`

## Arch Deployment

Systemd and nginx examples live in `packaging/systemd` and `packaging/nginx`.
See `docs/DEPLOYMENT.md` for the current Arch/AUR deployment checklist.

After installing the AUR package, initialize the admin account before starting
the service:

```bash
sudo humen-mcp init-admin --email <admin-email>
```

Release assets for the `-bin` package can be built with:

```bash
scripts/package-release.sh 0.1.2
```
