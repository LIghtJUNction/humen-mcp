#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
cd "${repo_root}"

cargo_version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
version="${1:-${cargo_version}}"
target="${TARGET:-x86_64-unknown-linux-gnu}"
out_dir="${OUT_DIR:-dist-release}"
name="humen-mcp-${version}-${target}"

if [[ -z "${cargo_version}" ]]; then
  echo "Could not read package version from Cargo.toml" >&2
  exit 1
fi

if [[ "${version}" != "${cargo_version}" ]]; then
  echo "Version mismatch: requested ${version}, but Cargo.toml is ${cargo_version}" >&2
  exit 1
fi

if [[ ! -f humen-mcp-webui/package.json ]]; then
  echo "humen-mcp-webui submodule is missing. Run: git submodule update --init humen-mcp-webui" >&2
  exit 1
fi

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
install -Dm644 packaging/systemd/humen-mcp-self-update.service "${out_dir}/${name}/packaging/systemd/humen-mcp-self-update.service"
install -Dm755 packaging/scripts/humen-mcp-self-update "${out_dir}/${name}/packaging/scripts/humen-mcp-self-update"
install -Dm440 packaging/sudoers/humen-mcp-self-update "${out_dir}/${name}/packaging/sudoers/humen-mcp-self-update"
install -Dm644 packaging/sysusers/humen-mcp.conf "${out_dir}/${name}/packaging/sysusers/humen-mcp.conf"
install -Dm644 packaging/tmpfiles/humen-mcp.conf "${out_dir}/${name}/packaging/tmpfiles/humen-mcp.conf"
install -Dm644 env.example "${out_dir}/${name}/env.example"

tar -C "${out_dir}" -czf "${out_dir}/${name}.tar.gz" "${name}"
sha256sum "${out_dir}/${name}.tar.gz"
