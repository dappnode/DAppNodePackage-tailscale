#!/bin/sh
set -eu

image=${1:?Usage: container-smoke.sh IMAGE}
container=
cleanup() {
    if [ -n "$container" ]; then
        docker rm -f -v "$container" > /dev/null
    fi
}
trap cleanup EXIT
trap 'exit 143' TERM
trap 'exit 130' INT

# No tailnet credentials or host network access. All firewall/sysctl changes
# remain inside this disposable container's network namespace.
container=$(docker run -d --network none --cap-add NET_ADMIN \
    --device /dev/net/tun:/dev/net/tun \
    --sysctl net.ipv4.ip_forward=1 --sysctl net.ipv6.conf.all.forwarding=1 \
    -e TS_AUTHKEY= -e TS_ROUTES=172.33.0.0/16,10.20.0.0/24 \
    -e TS_EXTRA_ARGS=--advertise-exit-node "$image")

ready=false
for attempt in 1 2 3 4 5 6 7 8 9 10; do
    if docker exec "$container" tailscale status --json > /dev/null 2>&1; then
        ready=true
        break
    fi
    sleep 1
done
if [ "$ready" != true ]; then
    docker logs "$container"
    exit 1
fi

docker exec "$container" sh -c '
    test "$(cat /proc/sys/net/ipv4/ip_forward)" = 1
    test "$(cat /proc/sys/net/ipv6/conf/all/forwarding)" = 1
    test "$TS_AUTH_ONCE" = true
    test "$TS_BOOT_TIMEOUT" = 1h
    test "$TS_DEBUG_FIREWALL_MODE" = auto
'
if docker exec "$container" /usr/local/bin/dappnode-healthcheck; then
    printf '%s\n' 'An unauthenticated daemon must not report healthy.' >&2
    exit 1
fi
docker stop -t 10 "$container" > /dev/null
exit_code=$(docker inspect --format '{{.State.ExitCode}}' "$container")
test "$exit_code" != 137
printf '%s\n' 'Container startup, forwarding, unauthenticated health, and clean shutdown passed.'
