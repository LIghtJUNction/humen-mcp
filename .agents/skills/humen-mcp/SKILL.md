---
name: humen-mcp
description: "Use when working in the humen-mcp repository: MCP endpoint behavior, ask_humen tool semantics, Rust backend, Bun web UI submodule, AUR packaging, or Arch deployment."
---

# humen-mcp

## Keep

- MCP endpoint: `/mcp`.
- Web UI route: `/mcp/` with the trailing slash.
- Core MCP tools:
  - `ask_humen` for blocking human replies.
  - `ask_humen_*_async` plus `read_humen_replies` for non-blocking requests.
  - `create_humen_task` and `list_humen_tasks` for AI-created human task lists.
  - `list_online_humens`, `search_humen_profiles`, `list_humen_tags`, `rate_humen`, and `report_humen` for human discovery and reputation.
- Tasks: choice, judgment, short text, image review, or explicit agent-provided steps.
- Backend: Rust; prefer established crates.
- Frontend: `humen-mcp-webui` submodule; uses Bun.
- Packaging: Arch/AUR install must keep the self-update systemd unit, helper script, sudoers rule, sysusers, tmpfiles, and packaged web dist in sync.
- Release examples: keep `examples/` aligned with the current MCP tool schemas in `src/mcp.rs`.

## Cleanup

- `target/` and `dist-release/` are ignored build outputs, not source.
- Old expanded release directories and tarballs under `dist-release/` can be deleted after upload.
- Do not delete dirty submodule changes unless the user explicitly asks.

## Verification

- Backend: `cargo check`.
- Examples: parse every `examples/*.json` file.
- Frontend: `bun install`; `bun run build` in `humen-mcp-webui`.
- Deploy: verify systemd and nginx still expose `/mcp`.
