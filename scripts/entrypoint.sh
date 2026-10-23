#!/bin/sh
set -eu

# With no key, containerboot already supports browser login. Keep its signal
# handling and persisted-state behavior unchanged on that path.
if [ -z "${TS_AUTHKEY:-}${TS_AUTH_KEY:-}" ]; then
    exec containerboot "$@"
fi

umask 077
workdir=$(mktemp -d)
mkfifo "$workdir/output"
child=
logger=

cleanup() {
    rm -rf "$workdir"
}

stop() {
    trap '' TERM INT
    if [ -n "$child" ]; then
        kill -TERM "$child" 2>/dev/null || :
        wait "$child" 2>/dev/null || :
    fi
    if [ -n "$logger" ]; then
        kill -TERM "$logger" 2>/dev/null || :
        wait "$logger" 2>/dev/null || :
    fi
    exit "$1"
}

trap cleanup EXIT
trap 'stop 143' TERM
trap 'stop 130' INT

# Forward logs immediately. Persist only a rejection marker, never an unbounded
# copy of daemon logs or a credential. Match explicit upstream auth errors so
# network failures and unrelated startup errors keep their original behavior.
(
    while IFS= read -r line || [ -n "$line" ]; do
        printf '%s\n' "$line"
        case "$line" in
            "backend error: invalid key:"*|"Error: invalid key:"*|*"Received error: invalid key:"*)
                : > "$workdir/key-rejected"
                ;;
        esac
    done < "$workdir/output"
) &
logger=$!

containerboot "$@" > "$workdir/output" 2>&1 &
child=$!
result=0
wait "$child" || result=$?
wait "$logger" || :

if [ "$result" -ne 0 ] && [ -f "$workdir/key-rejected" ]; then
    printf '%s\n' 'The configured Tailscale auth key was rejected. Falling back to browser login; open the login URL in these logs. Replace or clear the rejected key in the package configuration.'
    unset TS_AUTHKEY TS_AUTH_KEY
    cleanup
    trap - EXIT TERM INT
    exec containerboot "$@"
fi

exit "$result"
