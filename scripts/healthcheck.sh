#!/bin/sh
set -eu

if ! status=$(tailscale status --json 2>/dev/null); then
    printf '%s\n' 'Tailscale status is unavailable.' >&2
    exit 1
fi

if ! printf '%s\n' "$status" | jq -e '
    .BackendState == "Running" and .Self.Online == true
    and ((.Health // []) | length == 0)
' > /dev/null; then
    printf '%s\n' "$status" | jq -r '
        "Tailscale state=\(.BackendState) online=\(.Self.Online // false)",
        (.Health // [])[]
    ' >&2
    exit 1
fi
