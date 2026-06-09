# humen-mcp

Human-in-the-loop MCP server. Agents call MCP tools, `humen-mcp` turns the call into a human task envelope, a logged-in human answers it in the web UI, and the waiting MCP call receives the answer.

## What it provides

- MCP JSON-RPC endpoint at `/mcp`.
- Human workbench web UI served under `/mcp/`.
- Rust backend with REST APIs and WebSocket updates.
- Bun/Vite/React frontend in the `humen-mcp-webui` git submodule.
- AUR packages for Arch Linux:
  - `humen-mcp-git`: builds from GitHub source.
  - `humen-mcp-bin`: installs a GitHub Release tarball.
- Admin password login plus optional GitHub OAuth for non-admin humans.
- Live presence count and persisted active periods in the users JSON file.
- Request envelope lifecycle: pending, answered, expired, trash.

## Important paths

| Purpose | Path |
| --- | --- |
| MCP endpoint | `/mcp` |
| Web UI | `/mcp/` |
| Local backend bind | `127.0.0.1:8787` by default |
| Packaged web dist | `/usr/share/humen-mcp/web` |
| Service env file | `/etc/humen-mcp.env` |
| User/activity store | `/var/lib/humen-mcp/users.json` |
| systemd unit | `humen-mcp.service` |

`GET /mcp` is intentionally not the UI; it returns a method warning. Open the UI at `/mcp/` with the trailing slash.

## Local development

```bash
cp env.example .env
cargo run
```

Build the frontend inside the submodule:

```bash
cd humen-mcp-webui
bun install
bun run build
```

Then restart the backend. For local dev, `HUMEN_WEB_DIST=./humen-mcp-webui/dist` is fine.

Useful checks:

```bash
cargo test
cargo check
cd humen-mcp-webui && bun run build
curl -fsS http://127.0.0.1:8787/healthz
```

## MCP tools

Implemented MCP methods:

- `initialize`
- `notifications/initialized`
- `tools/list`
- `tools/call`

Current tools include:

- `ask_humen`
- `list_online_humens`
- `search_humen_profiles`
- `list_humen_tags`

### `ask_humen`

`ask_humen` accepts simple human tasks:

```json
{
  "kind": "choice|text|image_review|steps",
  "title": "Short task title",
  "prompt": "What the human should do",
  "choices": ["A", "B"],
  "image_url": "https://...",
  "image_base64": "iVBORw0KGgo...",
  "image_mime_type": "image/png",
  "steps": ["Open the site", "Read the SMS code"],
  "timeout_seconds": 60
}
```

Image review tasks may use either `image_url` or `image_base64`. `image_base64` may be raw base64 bytes or a full `data:image/...;base64,...` URL. When raw base64 is used, `image_mime_type` defaults to `image/png`.

`timeout_seconds` is agent-configurable. If omitted, the backend uses 60 seconds;
values are clamped by the server before the envelope is created. The backend
creates an envelope:

```json
{
  "id": "...",
  "title": "...",
  "prompt": "...",
  "kind": "text",
  "created_at": 1000,
  "timeout_seconds": 60,
  "expires_at": 1060
}
```

If the human does not answer in time, the request is removed from pending, added to trash, and the MCP caller receives a structured error:

```json
{
  "code": -32001,
  "message": "Human request timed out after 60 seconds",
  "data": {
    "request_id": "...",
    "title": "...",
    "timeout_seconds": 60,
    "expired_at": 1060,
    "suggestion": "Try again with a longer timeout or simplify the request."
  }
}
```

Example payloads are in `examples/`.

## HTTP and WebSocket API

Authenticated UI APIs:

- `GET /api/me`
- `GET /api/requests`
- `POST /api/requests/{id}/answer`
- `GET /api/trash`
- `POST /api/trash/clear`
- `GET /api/users/online`
- `GET /api/users/search?q=...`
- `GET /api/tags`
- `GET /api/ws`

Admin APIs:

- `GET /api/admin/users`
- `POST /api/admin/users`
- `POST /api/admin/users/{email}`
- `POST /api/admin/users/{email}/kick`
- `GET /api/admin/settings`
- `POST /api/admin/settings`

WebSocket events:

```json
{ "type": "request_created", "request": {} }
{ "type": "request_answered", "id": "...", "answer": {} }
{ "type": "request_expired", "id": "...", "expired_request": {} }
{ "type": "trash_cleaned", "removed_count": 1 }
{ "type": "presence_changed", "online_count": 1 }
```

## Configuration

`env.example` documents all supported variables. Important production values:

```bash
HUMEN_BIND=127.0.0.1:8787
HUMEN_PUBLIC_BASE_URL=https://your-domain.example/mcp
HUMEN_WEB_DIST=/usr/share/humen-mcp/web
HUMEN_USERS_FILE=/var/lib/humen-mcp/users.json
HUMEN_ADMIN_EMAIL=<admin-email>
HUMEN_ADMIN_PASSWORD=<generated-admin-password>
HUMEN_SESSION_SECRET=<generated-session-secret>
HUMEN_TRASH_RETENTION_SECONDS=604800
HUMEN_CLEANUP_INTERVAL_SECONDS=60
```

Production note: after AUR install, make sure `/etc/humen-mcp.env` uses the packaged web directory:

```bash
HUMEN_WEB_DIST=/usr/share/humen-mcp/web
```

If this is left as `./humen-mcp-webui/dist`, `https://your-domain.example/mcp/` will return 404 because the service runs from `/var/lib/humen-mcp`.

## Arch deployment

Install with an AUR helper as a normal user, not root:

```bash
paru -S humen-mcp-git
# or, after a GitHub Release exists:
paru -S humen-mcp-bin
```

On servers where the AUR user is `arch`, use a login shell:

```bash
sudo -iu arch paru -S humen-mcp-git
```

Avoid `sudo -u arch paru ...`; it can inherit root's working directory/git environment and fail with errors like `fatal: error reading '/root/.git'`.

Initialize or reset the admin account:

```bash
sudo humen-mcp init-admin --email <admin-email>
sudoedit /etc/humen-mcp.env
sudo systemctl enable --now humen-mcp.service
```

The `init-admin` command writes `/etc/humen-mcp.env`, generates a new session secret, and prints the generated admin password. Save that password in your password manager; do not commit it.

After editing `/etc/humen-mcp.env`:

```bash
sudo systemctl restart humen-mcp.service
```

## Nginx reverse proxy

Include `packaging/nginx/humen-mcp.conf` in the HTTPS server block for your domain.

Required behavior:

- `location = /mcp` proxies to backend `/mcp` for JSON-RPC.
- `location /mcp/` proxies to backend `/` for the web UI and static assets.
- `location /` may redirect to `/mcp/`.

Example verification:

```bash
sudo nginx -t
sudo systemctl reload nginx
curl -i https://your-domain.example/mcp/
curl -fsS https://your-domain.example/mcp/api/auth/config
```

A working `/mcp/` response should be `HTTP 200` with the web UI HTML.

## Verification checklist

After install or upgrade:

```bash
pacman -Q humen-mcp-git humen-mcp-bin 2>/dev/null || true
humen-mcp --version
systemctl is-active humen-mcp.service
curl -fsS http://127.0.0.1:8787/healthz
curl -fsS http://127.0.0.1:8787/mcp \
  -H 'content-type: application/json' \
  --data @examples/mcp-tools-list.json
curl -i https://your-domain.example/mcp/
```

Expected:

- service is `active`
- health returns `{"ok":true}`
- `tools/list` includes `ask_humen`
- `/mcp/` returns the web UI HTML, not 404

## Release packaging

Build a binary release tarball:

```bash
scripts/package-release.sh <version>
```

Or publish through GitHub Actions:

```bash
git tag v<version>
git push origin v<version>
```

The `Release` workflow can also be run manually from GitHub Actions with a
`version` input. It builds the Rust binary and web UI, uploads the tarball plus
`.sha256` as workflow artifacts, and creates or updates GitHub Release
`v<version>`.

The tarball is written to `dist-release/` and contains:

- `humen-mcp`
- `web/`
- `packaging/systemd/humen-mcp.service`
- `packaging/sysusers/humen-mcp.conf`
- `packaging/tmpfiles/humen-mcp.conf`
- `env.example`

For `humen-mcp-bin`, upload the tarball to GitHub Release `v<version>`, then update `aur/humen-mcp-bin/PKGBUILD` and `.SRCINFO` with the new version and sha256.

## Troubleshooting

### `https://domain/mcp/` returns 404

Check backend root first:

```bash
curl -i http://127.0.0.1:8787/
```

If it is also 404, inspect the running service environment:

```bash
pid=$(systemctl show -p MainPID --value humen-mcp.service)
sudo tr '\0' '\n' < /proc/$pid/environ | grep HUMEN_WEB_DIST
```

Fix `/etc/humen-mcp.env`:

```bash
HUMEN_WEB_DIST=/usr/share/humen-mcp/web
```

Then restart:

```bash
sudo systemctl restart humen-mcp.service
```

### AUR build says it is reading `/root/.git`

Run `paru` through a login shell for the normal AUR user:

```bash
sudo -iu arch bash -lc 'unset GIT_DIR GIT_WORK_TREE; cd ~; paru -S humen-mcp-git'
```

### Admin login still shows `<admin-email>`

The admin account was not initialized. Run:

```bash
sudo humen-mcp init-admin --email <admin-email>
sudo systemctl restart humen-mcp.service
```
