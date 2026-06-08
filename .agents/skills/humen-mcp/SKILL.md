# humen-mcp Project Skill

Use this skill when working in this repository on the human-in-the-loop MCP server, web UI, AUR packaging, or Arch deployment.

## Invariants

- MCP endpoint remains mounted at `/mcp`.
- The primary MCP tool is named `ask_humen`.
- Human tasks should stay simple: choice, short text, image review, or following explicit agent-provided steps.
- Frontend source lives in the `humen-mcp-webui` git submodule and uses Bun.
- Backend is Rust and should prefer established crates over hand-rolled protocol or web primitives.

## Verification

- Run `cargo check` after backend changes.
- Run `bun install` and `bun run build` inside `humen-mcp-webui` after frontend changes.
- For deployment changes, verify the systemd unit and nginx path still expose the app under `/mcp`.

