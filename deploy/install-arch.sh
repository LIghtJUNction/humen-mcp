#!/usr/bin/env bash
set -euo pipefail

install -Dm755 target/release/humen-mcp /usr/bin/humen-mcp
install -Dm644 packaging/systemd/humen-mcp.service /etc/systemd/system/humen-mcp.service
install -Dm640 env.example /etc/humen-mcp.env

systemctl daemon-reload
systemctl enable --now humen-mcp.service

