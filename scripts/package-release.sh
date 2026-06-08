#!/usr/bin/env bash
set -euo pipefail

version="${1:-0.1.0}"
target="${TARGET:-x86_64-unknown-linux-gnu}"
out_dir="${OUT_DIR:-dist-release}"
name="humen-mcp-${version}-${target}"

rm -rf "${out_dir:?}/${name}" "${out_dir:?}/${name}.tar.gz"
mkdir -p "${out_dir}/${name}/web"

(
  cd humen-mcp-webui
  bun install --frozen-lockfile
  bun run build
)

cargo build --release --locked

install -Dm755 target/release/humen-mcp "${out_dir}/${name}/humen-mcp"
cp -a humen-mcp-webui/dist/. "${out_dir}/${name}/web/"
install -Dm644 packaging/systemd/humen-mcp.service "${out_dir}/${name}/packaging/systemd/humen-mcp.service"
install -Dm644 packaging/sysusers/humen-mcp.conf "${out_dir}/${name}/packaging/sysusers/humen-mcp.conf"
install -Dm644 packaging/tmpfiles/humen-mcp.conf "${out_dir}/${name}/packaging/tmpfiles/humen-mcp.conf"
install -Dm644 env.example "${out_dir}/${name}/env.example"

tar -C "${out_dir}" -czf "${out_dir}/${name}.tar.gz" "${name}"
sha256sum "${out_dir}/${name}.tar.gz"

