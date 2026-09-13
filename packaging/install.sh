#!/usr/bin/env bash
# Install Pitwall for one user. Application files are installed under the
# user's XDG paths; the SQLite database and state artifact are never touched.
set -euo pipefail

die() { printf 'pitwall install: error: %s\n' "$*" >&2; exit 1; }
info() { printf 'pitwall install: %s\n' "$*"; }

repo_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
home_dir=${HOME:?HOME is required}
bin_dir="$home_dir/.local/bin"
plugin_dir=${XDG_CONFIG_HOME:-"$home_dir/.config"}/omarchy/plugins/dev.pitwall
plugin_marker="$plugin_dir/.pitwall-managed"
systemd_dir=${XDG_CONFIG_HOME:-"$home_dir/.config"}/systemd/user
binary="$repo_dir/target/release/pitwall"
build=true
enable_timer=false
enable_plugin=false
uninstall=false
manage_systemd=true

usage() {
  cat <<'EOF'
Usage: packaging/install.sh [OPTIONS]

Install Pitwall into the current user's XDG directories.

Options:
  --no-build       Use an existing target/release/pitwall binary
  --binary PATH    Install this already-built binary (implies --no-build)
  --enable-timer   Enable and start the optional 30-second user timer
  --enable-plugin  Enable dev.pitwall when omarchy is available
  --no-systemd     Do not contact the user systemd manager (test/helper use)
  --uninstall      Remove Pitwall application files, preserving user data
  -h, --help       Show this help

The database and state file remain under XDG_DATA_HOME (or
~/.local/share)/pitwall. Use the explicit data-directory commands if you
want to remove that history separately.
EOF
}

while (($#)); do
  case "$1" in
    --no-build) build=false ;;
    --binary)
      (($# >= 2)) || die "--binary requires a path"
      binary=$2; build=false; shift
      ;;
    --enable-timer) enable_timer=true ;;
    --enable-plugin) enable_plugin=true ;;
    --no-systemd) manage_systemd=false ;;
    --uninstall) uninstall=true ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option '$1' (use --help)" ;;
  esac
  shift
done

if [[ "$uninstall" == true ]]; then
  if [[ "$manage_systemd" == true ]] && command -v systemctl >/dev/null 2>&1; then
    systemctl --user disable --now pitwall-snapshot.timer >/dev/null 2>&1 || true
    systemctl --user daemon-reload >/dev/null 2>&1 || true
  fi
  rm -f "$bin_dir/pitwall" \
    "$systemd_dir/pitwall.service" \
    "$systemd_dir/pitwall-snapshot.timer"
  if [[ -f "$plugin_marker" ]]; then
    rm -rf "$plugin_dir"
  else
    info "left existing plugin directory untouched (not marked as Pitwall-managed): $plugin_dir"
  fi
  info "removed application files"
  info "preserved user data under ${XDG_DATA_HOME:-"$home_dir/.local/share"}/pitwall"
  info "if enabled, remove dev.pitwall from the Omarchy bar with: omarchy plugin disable dev.pitwall"
  exit 0
fi

if [[ "$build" == true ]]; then
  command -v cargo >/dev/null 2>&1 || die "cargo is required to build; pass --binary PATH to install an existing build"
  info "building release binary"
  cargo build --release --manifest-path "$repo_dir/Cargo.toml" || die "release build failed"
fi
[[ -x "$binary" ]] || die "binary is missing or not executable: $binary"
[[ -d "$repo_dir/plugin/dev.pitwall" ]] || die "plugin source directory is missing: $repo_dir/plugin/dev.pitwall"

# Install application files one class at a time. Existing user data is never
# in these paths, and copying into the plugin directory keeps upgrades
# idempotent without deleting unrelated files in its parent directory.
install -Dm755 "$binary" "$bin_dir/pitwall" || die "could not install binary at $bin_dir/pitwall"
mkdir -p "$plugin_dir" || die "could not create plugin directory $plugin_dir"
cp -a "$repo_dir/plugin/dev.pitwall/." "$plugin_dir/" || die "could not install Omarchy plugin"
printf 'managed-by=pitwall\n' > "$plugin_marker" || die "could not mark plugin installation"
install -Dm644 "$repo_dir/packaging/pitwall.service" "$systemd_dir/pitwall.service" || die "could not install systemd service"
install -Dm644 "$repo_dir/packaging/pitwall-snapshot.timer" "$systemd_dir/pitwall-snapshot.timer" || die "could not install systemd timer"

if [[ "$manage_systemd" == true ]] && command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload >/dev/null 2>&1 || \
    info "systemd user manager unavailable; units were installed but not reloaded"
fi

if [[ "$enable_timer" == true ]]; then
  [[ "$manage_systemd" == true ]] && command -v systemctl >/dev/null 2>&1 || die "--enable-timer requires systemctl"
  systemctl --user enable --now pitwall-snapshot.timer || \
    die "could not enable the user timer; inspect: systemctl --user status pitwall-snapshot.timer"
fi

if [[ "$enable_plugin" == true ]]; then
  command -v omarchy >/dev/null 2>&1 || die "--enable-plugin requires the omarchy command"
  omarchy plugin enable dev.pitwall --after omarchy.agents || \
    die "could not enable dev.pitwall; the plugin files remain installed"
fi

info "installed binary: $bin_dir/pitwall"
info "installed plugin: $plugin_dir"
info "installed user units: $systemd_dir"
info "user data is created on first snapshot under ${XDG_DATA_HOME:-"$home_dir/.local/share"}/pitwall"
if [[ "$enable_timer" != true ]]; then
  info "timer is installed but disabled; enable with: systemctl --user enable --now pitwall-snapshot.timer"
fi
if [[ "$enable_plugin" != true ]]; then
  info "plugin is installed but not enabled; enable with: omarchy plugin enable dev.pitwall --after omarchy.agents"
fi
