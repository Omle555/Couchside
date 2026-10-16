#!/usr/bin/env bash
# Builds and installs Couchside on Debian or Ubuntu (for example inside a Proxmox LXC
# container), as a systemd service that starts on boot. Run as root from a copy of the
# Couchside repository. Running it again updates Couchside and keeps its settings and database.
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONFIG_DIR=/etc/couchside
DATA_DIR=/var/lib/couchside
BIN=/usr/local/bin/couchside
AVAHI_SERVICES=/etc/avahi/services

say() { printf '\n==> %s\n' "$*"; }
die() { printf '\nERROR: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" -eq 0 ] || die "run this as root"
[ -f "$SRC/Cargo.toml" ] || die "run this from a copy of the Couchside repository"

say "Installing packages (ffmpeg, avahi-daemon, build tools)"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends \
    ca-certificates curl build-essential pkg-config ffmpeg avahi-daemon acl >/dev/null

if ! command -v cargo >/dev/null && [ ! -x /root/.cargo/bin/cargo ]; then
    say "Installing the Rust toolchain"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none
fi
export PATH="/root/.cargo/bin:$PATH"

say "Building Couchside (release)"
cd "$SRC"
rustup toolchain install --profile minimal >/dev/null   # the version in rust-toolchain.toml
cargo build --release --locked -p couchside-server

say "Installing"
id couchside >/dev/null 2>&1 || useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin couchside
install -d -o couchside -g couchside -m 0750 "$DATA_DIR"
install -d -m 0755 "$CONFIG_DIR"
if [ ! -f "$CONFIG_DIR/config.toml" ]; then
    sed -e 's|^bind = "127.0.0.1"|bind = "0.0.0.0"|' \
        -e "s|^# data_dir = .*|data_dir = '$DATA_DIR'|" \
        "$SRC/config.example.toml" > "$CONFIG_DIR/config.toml"
fi
chown root:couchside "$CONFIG_DIR/config.toml"
chmod 0640 "$CONFIG_DIR/config.toml"
# Couchside writes its own Avahi service file while it runs (network discovery).
install -d -m 0755 "$AVAHI_SERVICES"
setfacl -m u:couchside:rwx "$AVAHI_SERVICES"

systemctl stop couchside 2>/dev/null || true
install -m 0755 "$SRC/target/release/couchside" "$BIN"
install -m 0644 "$SRC/deploy/linux/couchside.service" /etc/systemd/system/couchside.service
systemctl daemon-reload
systemctl enable --now avahi-daemon >/dev/null
systemctl enable --now couchside

say "Couchside is running: $(systemctl is-active couchside)"
echo "    Settings: $CONFIG_DIR/config.toml   Data and logs: $DATA_DIR"
echo "    Logs:     journalctl -u couchside -f"
