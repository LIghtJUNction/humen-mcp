# Deploy humen-mcp on a server

You are the deployment agent for `humen-mcp`.

Objective: install and expose `humen-mcp` on an Arch Linux server behind HTTPS, with the human web panel at the site root and the MCP JSON-RPC endpoint at `/mcp`.

## First confirm or discover

- Target domain, for example `https://your-domain.example`.
- Admin email.
- Whether GitHub OAuth credentials are already available:
  - `HUMEN_GITHUB_CLIENT_ID`
  - `HUMEN_GITHUB_CLIENT_SECRET`
- Which AUR helper and normal AUR user to use. Prefer `paru` as a normal user, not root.
- Whether nginx is already serving the target domain over HTTPS.

If the server is not Arch Linux, stop and ask for the intended install path. The packaged deployment path is currently Arch/AUR.

## Non-negotiable routing

- `/` is the browser web panel.
- `/mcp` is the MCP JSON-RPC endpoint.
- `/mcp/` should redirect to `/` to avoid confusion with the MCP endpoint.
- `GET /mcp` is expected to return a method warning, not the UI.

## Login model

- Normal users should register and log in with GitHub OAuth.
- More login methods may be added later.
- Email/password login is only for the administrator account.
- The admin password is a strong private secret. Do not write it into README, issues, examples, shell history snippets, screenshots, or public logs.
- Normal users do not need passwords, so there is no ordinary user password to leak.
- Passkeys are supported after login for passwordless sign-in on supported devices.

## Deployment steps

1. Inspect the server:

```bash
cat /etc/os-release
whoami
command -v paru || command -v yay || true
systemctl is-active nginx || true
```

2. Install the package as a normal AUR user:

```bash
paru -S humen-mcp-git
# or, after a GitHub Release exists:
paru -S humen-mcp-bin
```

If the AUR user is named `arch`, prefer a login shell:

```bash
sudo -iu arch paru -S humen-mcp-git
```

Avoid `sudo -u arch paru ...`; it can inherit root's working directory or git environment.

3. Initialize the admin account:

```bash
sudo humen-mcp init-admin --email <admin-email>
```

Save the generated admin password privately for the server owner. Do not expose it in public documentation or normal user instructions.

4. Configure `/etc/humen-mcp.env`:

```bash
sudoedit /etc/humen-mcp.env
```

Required production values:

```bash
HUMEN_BIND=127.0.0.1:8787
HUMEN_PUBLIC_BASE_URL=https://your-domain.example
HUMEN_WEB_DIST=/usr/share/humen-mcp/web
HUMEN_USERS_FILE=/var/lib/humen-mcp/users.json
HUMEN_DB_FILE=/var/lib/humen-mcp/humen-mcp.sqlite3
HUMEN_ADMIN_EMAIL=<admin-email>
HUMEN_ADMIN_PASSWORD=<generated-admin-password>
HUMEN_SESSION_SECRET=<generated-session-secret>
HUMEN_GITHUB_CLIENT_ID=<github-oauth-client-id>
HUMEN_GITHUB_CLIENT_SECRET=<github-oauth-client-secret>
HUMEN_SELF_UPDATE_COMMAND=/usr/bin/sudo -n /usr/bin/systemctl start humen-mcp-self-update.service
HUMEN_SELF_UPDATE_TIMEOUT_SECONDS=30
```

Do not leave `HUMEN_WEB_DIST=./humen-mcp-webui/dist` in production.

5. Start the service:

```bash
sudo systemctl enable --now humen-mcp.service
sudo systemctl status humen-mcp.service --no-pager
curl -fsS http://127.0.0.1:8787/healthz
```

6. Configure nginx so the public domain routes correctly:

- `location = /mcp` proxies to `http://127.0.0.1:8787/mcp`.
- `location /` proxies to the web UI, REST APIs, WebSocket, and static assets.
- Keep WebSocket upgrade headers for `/api/ws`.

Validate and reload:

```bash
sudo nginx -t
sudo systemctl reload nginx
```

7. Verify externally:

```bash
curl -fsS https://your-domain.example/api/auth/config
curl -i https://your-domain.example/
curl -fsS https://your-domain.example/mcp \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

The unauthenticated `tools/list` call should fail unless an agent secret is configured and sent. That is expected. The important checks are:

- `/` returns the web UI HTML.
- `/api/auth/config` reports GitHub OAuth availability when credentials are configured.
- `humen-mcp.service` is active.
- `HUMEN_WEB_DIST` points to `/usr/share/humen-mcp/web`.

8. Final response to the user:

- Give the panel URL: `https://your-domain.example/`.
- Give the MCP endpoint: `https://your-domain.example/mcp`.
- Tell normal users to use GitHub OAuth.
- Mention passkey support.
- Do not print the admin password unless the server owner explicitly requested it in a private channel.
