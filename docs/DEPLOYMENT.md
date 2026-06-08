# Deployment

Target: Arch Linux server `archczy`, served under `https://xxx.yyy/mcp`.

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
- `/usr/lib/sysusers.d/humen-mcp.conf`
- `/usr/lib/tmpfiles.d/humen-mcp.conf`
- `/etc/humen-mcp.env`

## Configure

Edit `/etc/humen-mcp.env`:

```bash
HUMEN_BIND=127.0.0.1:8787
HUMEN_PUBLIC_BASE_URL=https://xxx.yyy/mcp
HUMEN_WEB_DIST=/usr/share/humen-mcp/web
HUMEN_ADMIN_EMAIL=you@example.com
HUMEN_ADMIN_PASSWORD=change-me
HUMEN_SESSION_SECRET=use-a-long-random-secret
```

Then:

```bash
systemctl enable --now humen-mcp.service
systemctl status humen-mcp.service
curl http://127.0.0.1:8787/healthz
```

## Nginx

Include `packaging/nginx/humen-mcp.conf` in the `xxx.yyy` server block and reload nginx:

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

Open `https://xxx.yyy/mcp/`, log in, and confirm the sidebar shows the live online count.
