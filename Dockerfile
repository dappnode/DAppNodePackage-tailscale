ARG UPSTREAM_VERSION
FROM alpine:3.23.7 AS ca-trust
RUN apk add --no-cache 'ca-certificates>=20261020' 'ca-certificates-bundle>=20261020'
RUN find /usr/share/ca-certificates -type f -name '*.crt' | sed 's|/usr/share/ca-certificates/||' | sort > /etc/ca-certificates.conf

FROM tailscale/tailscale:${UPSTREAM_VERSION}

ENV TS_STATE_DIR=/var/lib/tailscale
ENV TS_USERSPACE=false
ENV TS_AUTH_ONCE=true
ENV TS_BOOT_TIMEOUT=1h
ENV TS_DEBUG_FIREWALL_MODE=auto

RUN apk add --no-cache jq

# Replace the complete system trust store, including individual hashed certificates.
RUN rm -rf /etc/ssl/certs /usr/share/ca-certificates
COPY --from=ca-trust /etc/ssl/certs/ /etc/ssl/certs/
COPY --from=ca-trust /usr/share/ca-certificates/ /usr/share/ca-certificates/
COPY --from=ca-trust /etc/ca-certificates.conf /etc/ca-certificates.conf
RUN ln -sf certs/ca-certificates.crt /etc/ssl/cert.pem

COPY --chmod=755 scripts/entrypoint.sh /usr/local/bin/dappnode-entrypoint
COPY --chmod=755 scripts/healthcheck.sh /usr/local/bin/dappnode-healthcheck

HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 CMD ["/usr/local/bin/dappnode-healthcheck"]
CMD ["/usr/local/bin/dappnode-entrypoint"]
