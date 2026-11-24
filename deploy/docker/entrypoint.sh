#!/bin/sh
# Start Voltrus with /data/config.yaml, creating it on first start from the
# image's default with a random auth_secret, so sessions survive restarts.
# Any arguments replace the default command (e.g. `voltrus project validate`).
set -eu

if [ "$#" -gt 0 ]; then
    exec "$@"
fi

CONFIG=/data/config.yaml
if [ ! -f "$CONFIG" ]; then
    secret=$(head -c 48 /dev/urandom | base64 | tr -d '\n/+=' | cut -c1-48)
    cp /etc/voltrus/config.yaml "$CONFIG"
    printf 'auth_secret: "%s"\n' "$secret" >> "$CONFIG"
    echo "voltrus: wrote $CONFIG with a new auth_secret"
fi

exec /usr/local/bin/voltrus "$CONFIG"
