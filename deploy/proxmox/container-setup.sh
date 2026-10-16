#!/usr/bin/env bash
# Finishes a Couchside LXC container after deploy/linux/install.sh: gives root a password for the
# Proxmox console, creates the first Couchside administrator, and shows Couchside's address on
# the console. Run inside the container as root; couchside-lxc.sh does this for you. Existing
# passwords and accounts are never changed.
set -euo pipefail

CREDENTIALS=/root/couchside-credentials.txt
CONFIG=/etc/couchside/config.toml
PORT="$(sed -n 's/^port *= *\([0-9]*\).*/\1/p' "$CONFIG" | head -n 1)"
PORT="${PORT:-8096}"
API="http://127.0.0.1:$PORT/api/v1"

new_password() { tr -dc 'A-Za-z0-9' </dev/urandom | head -c 16; }
note() { printf '%s\n' "$*" >>"$CREDENTIALS"; }

touch "$CREDENTIALS"
chmod 0600 "$CREDENTIALS"

# The Debian template leaves root without a password, so the console login can't be used.
if passwd -S root | awk '{exit !($2 == "L" || $2 == "NP")}'; then
    password="$(new_password)"
    echo "root:$password" | chpasswd
    note "Container console login:  root / $password"
fi

# Wait for Couchside to finish starting, then create the first administrator if none exists.
for _ in $(seq 1 60); do
    curl -fs "$API/health" >/dev/null 2>&1 && break
    sleep 1
done
if curl -fs "$API/setup" | grep -q '"needs_setup":true'; then
    password="$(new_password)"
    curl -fs -X POST "$API/setup" -H 'content-type: application/json' \
        -d "{\"username\":\"admin\",\"password\":\"$password\"}" >/dev/null
    note "Couchside web sign-in:    admin / $password"
fi

# Shown above the console login prompt; \4 is replaced with the container's IPv4 address.
cat >/etc/issue <<EOF
Debian GNU/Linux \\n \\l

  Couchside media server
  Open from any device on your network:  http://\\4:$PORT

EOF

# Shown after signing in on the console or over SSH.
cat >/etc/profile.d/couchside.sh <<EOF
echo
echo "  Couchside is \$(systemctl is-active couchside) at http://\$(hostname -I | awk '{print \$1}'):$PORT"
echo "  Passwords created at install: cat $CREDENTIALS"
echo "  Logs: journalctl -u couchside -f    Settings: $CONFIG"
echo
EOF
chmod 0644 /etc/profile.d/couchside.sh

echo
echo "  Open Couchside from any device on your network:"
echo "      http://$(hostname -I | awk '{print $1}'):$PORT"
if [ -s "$CREDENTIALS" ]; then
    echo
    echo "  Accounts (saved in $CREDENTIALS inside the container):"
    sed 's/^/      /' "$CREDENTIALS"
fi
echo
