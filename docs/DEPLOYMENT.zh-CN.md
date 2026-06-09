# 部署

语言：[English](DEPLOYMENT.md) | [简体中文](DEPLOYMENT.zh-CN.md)

目标：在 Arch Linux 服务器上部署，并通过 `https://your-domain.example/mcp` 提供服务。

公开实例：

| 用途 | URL |
| --- | --- |
| 人类工作台 | `https://humen.lmm.best/mcp/` |
| MCP 端点 | `https://humen.lmm.best/mcp` |

浏览器面板使用带尾斜杠的 `/mcp/`。MCP 客户端使用不带尾斜杠的 `/mcp`。

## 智能体辅助安装

拉取部署提示词，然后把完整输出交给负责操作服务器的智能体：

```bash
curl -fsSL https://raw.githubusercontent.com/LIghtJUNction/humen-mcp/main/docs/AGENT_DEPLOY_PROMPT.md
```

这个提示词会要求智能体检查主机、安装包、配置 systemd 和 nginx、验证 `/mcp`，并避免在公开输出中泄露管理员密钥。

## 登录模型

普通用户应通过 GitHub OAuth 注册和登录。后续可以继续添加其他登录方式。

邮箱密码登录只用于管理员账号。管理员密码是强私密信息，不要写进 README、示例、截图、issue 或面向用户的部署说明。

普通用户不需要密码，因此没有普通用户密码可泄露。登录后可以添加 passkey，在支持的设备上免密码登录。

## 软件包

源码 AUR 包是 `humen-mcp-git`，二进制包是 `humen-mcp-bin`。

```bash
paru -S humen-mcp-git
# 或
paru -S humen-mcp-bin
```

仓库子目录：

- `aur/humen-mcp-git`：源码包，构建 Rust 后端和 Bun Web UI。
- `aur/humen-mcp-bin`：二进制包，安装 GitHub Release tarball。

包会安装：

- `/usr/bin/humen-mcp`
- `/usr/share/humen-mcp/web`
- `/usr/lib/systemd/system/humen-mcp.service`
- `/usr/lib/systemd/system/humen-mcp-self-update.service`
- `/usr/lib/humen-mcp/humen-mcp-self-update`
- `/usr/lib/sysusers.d/humen-mcp.conf`
- `/usr/lib/tmpfiles.d/humen-mcp.conf`
- `/etc/sudoers.d/humen-mcp-self-update`
- `/etc/humen-mcp.env`

## 配置

先初始化管理员登录：

```bash
sudo humen-mcp init-admin --email <admin-email>
```

该命令会写入 `/etc/humen-mcp.env`，生成 session secret，并打印生成的管理员密码。随后编辑 `/etc/humen-mcp.env`：

```bash
HUMEN_BIND=127.0.0.1:8787
HUMEN_PUBLIC_BASE_URL=https://your-domain.example/mcp
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
HUMEN_PLUGIN_DIR=/etc/humen-mcp/plugins
```

只有配置的管理员账号能用邮箱密码登录。GitHub OAuth 在 `HUMEN_GITHUB_CLIENT_ID` 和 `HUMEN_GITHUB_CLIENT_SECRET` 配好前是关闭的；启用后，GitHub 登录也是普通用户注册入口。

打包的 systemd unit 会配置自更新命令。如果你的 AUR 用户不是 `arch`，编辑 updater 环境：

```bash
sudoedit /etc/humen-mcp-update.env
```

```bash
HUMEN_UPDATE_AUR_USER=<aur-user>
HUMEN_UPDATE_HELPER=paru
HUMEN_UPDATE_PACKAGE=humen-mcp-bin
```

updater 是非交互式的。确认 AUR 用户能无密码运行安装步骤：

```bash
sudo -iu <aur-user> sudo -n true
```

启动服务：

```bash
systemctl enable --now humen-mcp.service
systemctl status humen-mcp.service
curl http://127.0.0.1:8787/healthz
```

## nginx

把 `packaging/nginx/humen-mcp.conf` include 到你的 HTTPS server block，然后 reload nginx：

```bash
nginx -t
systemctl reload nginx
```

必须满足：

- `location = /mcp` 代理到后端 `/mcp`，用于 MCP JSON-RPC。
- `location /mcp/` 代理到 Web UI 和静态资源。

## 插件目录

如果启用社区插件，创建插件目录并放入 JSON/TOML manifest：

```bash
sudo install -d -o humen-mcp -g humen-mcp /etc/humen-mcp/plugins
sudo cp examples/community-release-plugin.json /etc/humen-mcp/plugins/
sudo systemctl restart humen-mcp.service
```

然后通过 MCP 工具确认加载：

```bash
curl -s http://127.0.0.1:8787/mcp \
  -H 'content-type: application/json' \
  -H "x-humen-agent-secret: ${HUMEN_AGENT_SECRET}" \
  --data @examples/mcp-list-plugins.json
```

## 验证

```bash
curl -s http://127.0.0.1:8787/mcp \
  -H 'content-type: application/json' \
  --data @examples/mcp-tools-list.json
```

打开 `https://your-domain.example/mcp/`，登录后确认侧边栏显示在线人数。
