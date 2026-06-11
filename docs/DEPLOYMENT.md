# Deployment

Language: [English](DEPLOYMENT.md) | [简体中文](DEPLOYMENT.zh-CN.md)

Target: Arch Linux server, served under `https://your-domain.example/`.

Public hosted instance:

| Purpose | URL |
| --- | --- |
| Human workbench | `https://humen.lmm.best/` |
| MCP endpoint | `https://humen.lmm.best/mcp` |

The browser panel uses the site root. MCP clients use `/mcp`.

## Agent-assisted install

Fetch the deployment prompt, then send its full output to the agent that will
operate on the server:

```bash
curl -fsSL https://raw.githubusercontent.com/LIghtJUNction/humen-mcp/main/docs/AGENT_DEPLOY_PROMPT.md
```

The prompt tells the agent to inspect the host, install the package, configure
systemd and nginx, verify `/mcp`, and keep admin secrets out of public output.

## Login model

Normal users should register and log in with GitHub OAuth. More login methods
can be added later.

Email/password login is only for the administrator account. The admin password
is a strong private secret; do not put it in README, examples, screenshots,
issues, or user-facing setup text.

Normal users do not need passwords, so there is no ordinary user password to
leak. Passkeys are supported after login for passwordless sign-in on supported
devices.

## Package

The source AUR package is `humen-mcp-git`; the binary package is `humen-mcp-bin`.

```bash
paru -S humen-mcp-git
# or
paru -S humen-mcp-bin
```

Repository submodules:

- `aur/humen-mcp-git`: source package, builds the Rust backend and Bun web UI.
- `aur/humen-mcp-bin`: binary package, installs the GitHub Release tarball.

The package installs:

- `/usr/bin/humen-mcp`
- `/usr/share/humen-mcp/web`
- `/usr/lib/systemd/system/humen-mcp.service`
- `/usr/lib/systemd/system/humen-mcp-self-update.service`
- `/usr/lib/humen-mcp/humen-mcp-self-update`
- `/usr/lib/sysusers.d/humen-mcp.conf`
- `/usr/lib/tmpfiles.d/humen-mcp.conf`
- `/etc/sudoers.d/humen-mcp-self-update`
- `/etc/humen-mcp.env`

## Configure

Initialize the admin login first:

```bash
sudo humen-mcp init-admin --email <admin-email>
```

The command writes `/etc/humen-mcp.env`, generates a session secret, and prints the generated admin password. Then edit `/etc/humen-mcp.env` for the public URL:

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

Only the configured admin account can use email/password login. GitHub OAuth is
disabled until `HUMEN_GITHUB_CLIENT_ID` and `HUMEN_GITHUB_CLIENT_SECRET` are
configured; once enabled, GitHub login is also the registration path for
non-admin humans.

The packaged systemd unit configures the self-update command automatically. If
your AUR user is not `arch`, override the updater service environment:

```bash
sudoedit /etc/humen-mcp-update.env
```

```bash
HUMEN_UPDATE_AUR_USER=<aur-user>
HUMEN_UPDATE_HELPER=paru
HUMEN_UPDATE_PACKAGE=humen-mcp-bin
```

The updater is non-interactive. Make sure the AUR user can run the package
installation step without a password prompt, for example:

```bash
sudo -iu <aur-user> sudo -n true
```

Then:

```bash
systemctl enable --now humen-mcp.service
systemctl status humen-mcp.service
curl http://127.0.0.1:8787/healthz
```

## Nginx

Include `packaging/nginx/humen-mcp.conf` in your HTTPS server block and reload nginx:

```bash
nginx -t
systemctl reload nginx
```

## Verification

```bash
curl -s http://127.0.0.1:8787/mcp \
  -H 'content-type: application/json' \
  --data @examples/mcp-tools-list.json
```

Open `https://your-domain.example/`, log in, and confirm the sidebar shows the live online count.
