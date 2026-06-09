# Deployment

Target: Arch Linux server `archczy`, served under `https://your-domain.example/mcp`.

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
HUMEN_PUBLIC_BASE_URL=https://your-domain.example/mcp
HUMEN_WEB_DIST=/usr/share/humen-mcp/web
HUMEN_USERS_FILE=/var/lib/humen-mcp/users.json
HUMEN_ADMIN_EMAIL=<admin-email>
HUMEN_ADMIN_PASSWORD=<generated-admin-password>
HUMEN_SESSION_SECRET=<generated-session-secret>
HUMEN_SELF_UPDATE_COMMAND=/usr/bin/sudo -n /usr/bin/systemctl start humen-mcp-self-update.service
HUMEN_SELF_UPDATE_TIMEOUT_SECONDS=30
```

Only the configured admin account can use email/password login. GitHub OAuth is disabled until `HUMEN_GITHUB_CLIENT_ID` and `HUMEN_GITHUB_CLIENT_SECRET` are configured; once enabled, GitHub login is also the registration path for non-admin humans.

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

Open `https://your-domain.example/mcp/`, log in, and confirm the sidebar shows the live online count.
