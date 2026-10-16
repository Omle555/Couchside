#!/usr/bin/env bash
# Finishes a Couchside LXC container after deploy/linux/install.sh: creates the default login
# for the Proxmox console and for Couchside, and shows Couchside's address on the console. Run
# inside the container as root; couchside-lxc.sh does this for you. Existing passwords and
# accounts are never changed.
#
# Default login (both the console and Couchside): Admin / sidecouch. Change it with
# ADMIN_USER and ADMIN_PASSWORD (at least 6 characters).
set -euo pipefail

ADMIN_USER="${ADMIN_USER:-Admin}"
ADMIN_PASSWORD="${ADMIN_PASSWORD:-sidecouch}"
CONFIG=/etc/couchside/config.toml
PORT="$(sed -n 's/^port *= *\([0-9]*\).*/\1/p' "$CONFIG" | head -n 1)"
PORT="${PORT:-8096}"
API="http://127.0.0.1:$PORT/api/v1"

# Console login. The Debian template leaves root without a password, so it gets the same one.
export DEBIAN_FRONTEND=noninteractive
command -v sudo >/dev/null || apt-get install -y -qq --no-install-recommends sudo >/dev/null
if ! id "$ADMIN_USER" >/dev/null 2>&1; then
    # --badname allows the capital letter in "Admin".
    useradd --badname --create-home --shell /bin/bash --groups sudo "$ADMIN_USER"
    echo "$ADMIN_USER:$ADMIN_PASSWORD" | chpasswd
fi
if passwd -S root | awk '{exit !($2 == "L" || $2 == "NP")}'; then
    echo "root:$ADMIN_PASSWORD" | chpasswd
fi

# Wait for Couchside to finish starting, then create the first administrator if none exists.
for _ in $(seq 1 60); do
    curl -fs "$API/health" >/dev/null 2>&1 && break
    sleep 1
done
if curl -fs "$API/setup" | grep -q '"needs_setup":true'; then
    printf '{"username":"%s","password":"%s"}' "$ADMIN_USER" "$ADMIN_PASSWORD" |
        curl -fs -X POST "$API/setup" -H 'content-type: application/json' --data-binary @- >/dev/null
fi

# Shown above the console login prompt; \4 is replaced with the container's IPv4 address.
cat >/etc/issue <<EOF
Debian GNU/Linux \\n \\l

  Couchside media server
  Open from any device on your network:  http://\\4:$PORT
  Default login (here and in Couchside): $ADMIN_USER / $ADMIN_PASSWORD

EOF

# Shown after signing in on the console or over SSH.
cat >/etc/profile.d/couchside.sh <<EOF
echo
echo "  Couchside is \$(systemctl is-active couchside) at http://\$(hostname -I | awk '{print \$1}'):$PORT"
echo "  Logs: journalctl -u couchside -f    Settings: $CONFIG"
echo
EOF
chmod 0644 /etc/profile.d/couchside.sh

echo
echo "  Open Couchside from any device on your network:"
echo "      http://$(hostname -I | awk '{print $1}'):$PORT"
echo "  Default login for the console and Couchside: $ADMIN_USER / $ADMIN_PASSWORD"
echo "  (Change them later: 'passwd' on the console, Settings > Users in Couchside.)"
echo
