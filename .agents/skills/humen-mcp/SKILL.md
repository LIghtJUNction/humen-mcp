---
name: humen-mcp
description: Use when working in the humen-mcp repository: MCP endpoint behavior, ask_humen tool semantics, Rust backend, Bun web UI submodule, AUR packaging, or Arch deployment.
---

# humen-mcp

## Keep

- MCP endpoint: `/mcp`.
- MCP tool: `ask_humen`.
- Tasks: choice, short text, image review, or explicit agent-provided steps.
- Backend: Rust; prefer established crates.
- Frontend: `humen-mcp-webui` submodule; uses Bun.

## Verification

- Backend: `cargo check`.
- Frontend: `bun install`; `bun run build` in `humen-mcp-webui`.
- Deploy: verify systemd and nginx still expose `/mcp`.
