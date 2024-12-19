ARG UPSTREAM_VERSION
FROM tailscale/tailscale:${UPSTREAM_VERSION}

ENV TS_STATE_DIR=/var/lib/tailscale
ENV TS_USERSPACE=false
ENV TS_EXTRA_ARGS="--advertise-routes=172.33.0.0/16  --advertise-exit-node $EXTRA_OPTS"
