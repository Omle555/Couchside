#!/usr/bin/env bash
# Creates (or updates) a Couchside LXC container on a Proxmox VE host.
#
# Run as root on the Proxmox host, from a copy of the Couchside repository:
#   git clone https://github.com/Omle555/Couchside && bash Couchside/deploy/proxmox/couchside-lxc.sh
#
# Settings (environment variables, all optional):
#   CTID      container id                       (default: next free id)
#   CT_NAME   container host name                (default: couchside)
#   STORAGE   storage for the container disk     (default: local-lvm)
#   BRIDGE    network bridge                     (default: vmbr0)
#   DISK_GB   disk size in GB                    (default: 16; building needs about 6)
#   CORES     CPU cores                          (default: 4)
#   MEMORY    RAM in MB                          (default: 4096; building needs about 3000)
#   MEDIA     host folder to show inside as /media, e.g. /mnt/pve/nas/media (default: none)
#
# Running it again with the CTID of an existing Couchside container updates Couchside inside it
# to this copy of the code and keeps its settings, database and media mount.
set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CT_NAME="${CT_NAME:-couchside}"
STORAGE="${STORAGE:-local-lvm}"
BRIDGE="${BRIDGE:-vmbr0}"
DISK_GB="${DISK_GB:-16}"
CORES="${CORES:-4}"
MEMORY="${MEMORY:-4096}"
MEDIA="${MEDIA:-}"

say() { printf '\n==> %s\n' "$*"; }
die() { printf '\nERROR: %s\n' "$*" >&2; exit 1; }

command -v pct >/dev/null || die "run this on the Proxmox VE host (pct not found)"
[ "$(id -u)" -eq 0 ] || die "run this as root"
[ -f "$REPO_DIR/Cargo.toml" ] || die "run this from a copy of the Couchside repository"

create_container() {
    say "Finding the Debian 12 template"
    pveam update >/dev/null
    local template
    template="$(pveam available --section system | awk '/debian-12-standard/ {print $2}' | sort | tail -n 1)"
    [ -n "$template" ] || die "no Debian 12 template is available from pveam"
    if ! pveam list local | grep -q "$template"; then
        say "Downloading $template"
        pveam download local "$template"
    fi

    say "Creating container $CTID ($CT_NAME)"
    pct create "$CTID" "local:vztmpl/$template" \
        --hostname "$CT_NAME" \
        --unprivileged 1 \
        --features nesting=1 \
        --cores "$CORES" \
        --memory "$MEMORY" \
        --swap 1024 \
        --rootfs "$STORAGE:$DISK_GB" \
        --net0 "name=eth0,bridge=$BRIDGE,ip=dhcp" \
        --onboot 1 \
        --description "Couchside media server"
    if [ -n "$MEDIA" ]; then
        [ -d "$MEDIA" ] || die "MEDIA folder $MEDIA does not exist on this host"
        pct set "$CTID" --mp0 "$MEDIA,mp=/media,ro=1"
    fi
    pct start "$CTID"
    say "Waiting for the container's network"
    for _ in $(seq 1 30); do
        pct exec "$CTID" -- getent hosts deb.debian.org >/dev/null 2>&1 && return
        sleep 2
    done
    die "container $CTID has no network (check BRIDGE=$BRIDGE and DHCP)"
}

if [ -n "${CTID:-}" ] && pct status "$CTID" >/dev/null 2>&1; then
    say "Updating Couchside in existing container $CTID"
    pct status "$CTID" | grep -q running || pct start "$CTID"
else
    CTID="${CTID:-$(pvesh get /cluster/nextid)}"
    create_container
fi

say "Copying the Couchside source into the container"
archive="$(mktemp --suffix=.tar.gz)"
trap 'rm -f "$archive"' EXIT
tar -C "$REPO_DIR" --exclude=./target --exclude=./.git -czf "$archive" .
pct exec "$CTID" -- rm -rf /opt/couchside/src
pct exec "$CTID" -- mkdir -p /opt/couchside/src
pct push "$CTID" "$archive" /opt/couchside/src.tar.gz
pct exec "$CTID" -- tar -C /opt/couchside/src -xzf /opt/couchside/src.tar.gz
pct exec "$CTID" -- rm -f /opt/couchside/src.tar.gz

say "Building and installing Couchside inside the container (the first build takes a while)"
pct exec "$CTID" -- bash /opt/couchside/src/deploy/linux/install.sh

ip="$(pct exec "$CTID" -- hostname -I | awk '{print $1}')"
say "Done. Open http://${ip:-<container-ip>}:8096 from a device on your network."
if [ -z "$MEDIA" ]; then
    echo "    To add your media later: pct set $CTID --mp0 /path/on/host,mp=/media,ro=1 && pct reboot $CTID"
fi
