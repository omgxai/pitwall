#!/usr/bin/env bash
# Clean-room smoke test for the user-local installer. It uses a temporary HOME
# and runs the installed binary from outside the repository's working tree.
set -euo pipefail

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
[[ -x "$repo_dir/target/release/pitwall" ]] || {
  printf 'build first: cargo build --release\n' >&2
  exit 1
}

tmp_home=$(mktemp -d "${TMPDIR:-/tmp}/pitwall-install.XXXXXX")
trap 'rm -rf "$tmp_home"' EXIT
export HOME="$tmp_home"
export XDG_DATA_HOME="$tmp_home/data"
export XDG_CONFIG_HOME="$tmp_home/config"

"$repo_dir/packaging/install.sh" --binary "$repo_dir/target/release/pitwall" --no-systemd
[[ -x "$HOME/.local/bin/pitwall" ]]
[[ -f "$XDG_CONFIG_HOME/omarchy/plugins/dev.pitwall/manifest.json" ]]
[[ -f "$XDG_CONFIG_HOME/omarchy/plugins/dev.pitwall/.pitwall-managed" ]]
[[ -f "$XDG_CONFIG_HOME/systemd/user/pitwall.service" ]]
[[ -f "$XDG_CONFIG_HOME/systemd/user/pitwall-snapshot.timer" ]]
# The Chat branding asset reaches the first path `chat::branding_candidates`
# searches, and is the 256x256 PNG the render gate accepts (not the JPEG
# original, which has no in-tree decoder).
asset="$XDG_DATA_HOME/pitwall/assets/pitwallpixelart.png"
[[ -f "$asset" ]]
head -c 8 "$asset" | od -An -tx1 | tr -d ' \n' | grep -q '^89504e470d0a1a0a$'

version=$("$HOME/.local/bin/pitwall" --version)
[[ "$version" == pitwall\ * ]]
"$HOME/.local/bin/pitwall" snapshot --data-dir "$XDG_DATA_HOME/pitwall" >/dev/null
[[ -f "$XDG_DATA_HOME/pitwall/state.json" ]]
[[ -f "$XDG_DATA_HOME/pitwall/pitwall.db" ]]

"$repo_dir/packaging/install.sh" --uninstall --no-systemd
[[ ! -e "$HOME/.local/bin/pitwall" ]]
[[ ! -e "$XDG_CONFIG_HOME/omarchy/plugins/dev.pitwall" ]]
# Installed content goes; user data in the same tree stays.
[[ ! -e "$asset" ]]
[[ -f "$XDG_DATA_HOME/pitwall/state.json" ]]
[[ -f "$XDG_DATA_HOME/pitwall/pitwall.db" ]]
printf 'clean-room install: passed\n'
