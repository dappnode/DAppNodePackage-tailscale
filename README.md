# Tailscale for Dappnode

The `tailscale` service runs upstream Tailscale containerboot. The `reporter`
service is a small Rust binary that reads the daemon's structured LocalAPI status
and sends it to Dappmanager. It never reads container logs, starts a Tailscale
login, or changes daemon configuration.

## Signing in

Provide `TS_AUTHKEY` for automatic login, or leave it empty for browser login.
The image allows one hour for startup (`TS_BOOT_TIMEOUT=1h`); an explicit override
is respected. If the window expires, upstream containerboot exits and Docker
restarts the service. `TS_AUTH_ONCE=true` and the existing `state` volume retain
the device identity and reuse saved authentication across restarts.

With the accompanying Dappmanager integration, open **VPN → Tailscale** and
select **Sign in to Tailscale**. This button requires compatible Dappmanager UI
support. Older versions only expose the reporter's JSON in package sent data;
upstream Tailscale also continues printing its normal login instructions.

A rejected auth key does **not** automatically fall back to browser login.
Clear both `TS_AUTHKEY` and its upstream alias `TS_AUTH_KEY`, if set, then restart
the package. This integration does not replace upstream authentication handling.

After login, approve the subnet routes and configure tailnet DNS as described in
the [Dappnode Tailscale guide](https://docs.dappnode.io/docs/user/access-your-dappnode/vpn/tailscale).
Being authenticated and online does not prove subnet routing or DNS works.
This integration preserves the package's existing routing configuration.

## Containers and socket

Only `tailscale` mounts the persistent identity volume at `/var/lib/tailscale`.
A second volume shares `/run/tailscale/tailscaled.sock` with `reporter`. The
reporter mounts this directory read-only, drops all Linux capabilities, and
opens no listening port. It needs to run as root
to access the root-owned socket. It has no Docker socket or TUN access.

A read-only volume mount does **not** make Unix-socket requests read-only. Treat
the reporter as trusted code with LocalAPI access. Its only daemon request is
`GET /localapi/v0/status?peers=false`, with Host `local-tailscaled.sock`. Restarting
either service or replacing the socket is supported; no startup ordering is
required. Do not expose this socket to other packages.

The package uses Dappnode-supported Compose options. Its current validator does
not allow `read_only` (root filesystem) or `sysctls` service properties.

## Reporting contract

The reporter POSTs to:

```text
http://dappmanager.dappnode/data-send?key=tailscale.status
Content-Type: text/plain
```

The body is a compact JSON **string**, compatible with Dappmanager's existing
`data-send` endpoint and its 700-character limit:

```json
{"version":1,"state":"NeedsLogin","online":false,"authUrl":"https://login.tailscale.com/a/EXAMPLE","updatedAt":1793260800}
```

Fields:

| Field | Meaning |
| --- | --- |
| `version` | Contract version, currently `1`. |
| `state` | `NoState`, `NeedsLogin`, `NeedsMachineAuth`, `Stopped`, `Starting`, `Running`, or `Unavailable`. Unknown upstream states become `Unavailable`. |
| `online` | True only when state is `Running` and `Self.Online` is true. |
| `authUrl` | Current official HTTPS login URL when state is `NeedsLogin`; otherwise `null`. Invalid or oversized URLs are omitted as `null`, never truncated. |
| `updatedAt` | Unix epoch **seconds** when the status was observed, including each heartbeat. |

The reporter polls every two seconds, publishes changes immediately, and sends
unchanged status every 30 seconds. Each HTTP request has a three-second timeout.
After failed delivery it samples the daemon again and retries on the next poll;
it does not replay a queue of old URLs. Delivery failures never stop Tailscale.
LocalAPI failure publishes `Unavailable` with a null link. SIGTERM/SIGINT cancels
in-flight work and attempts a final `Unavailable` report for at most two seconds.

Dappmanager must expire reports after **90 seconds**, hide links when either
service is stopped, and independently validate the payload and login URL. Graceful
clearing cannot cover crashes, SIGKILL, or network outages. The timestamp in the
payload is not a Tailscale login-link expiration time.

Both HTTP clients bypass proxy environment variables and refuse redirects.
LocalAPI responses are limited to 64 KiB. Auth URLs are limited to 512 ASCII bytes
and must point to `https://login.tailscale.com/a/...`, without credentials,
fragments, whitespace, or control characters. Headscale/custom login servers are
not supported by this first contract. No auth key, peer list, status document,
or login URL is written to reporter logs. Dappmanager's current `packageSentData`
cache does persist the last payload; consumers must enforce the freshness and
display requirements above.

Reporter environment (normally no overrides are needed):

| Variable | Default |
| --- | --- |
| `TS_SOCKET` | `/run/tailscale/tailscaled.sock` |
| `DAPPMANAGER_URL` | `http://dappmanager.dappnode` (an HTTP origin with no path, query, or credentials) |

If changing `TS_SOCKET`, configure the same path in both services and keep it
inside the shared socket directory. Auth keys belong only in `tailscale`.

## Development

```sh
cargo fmt --manifest-path reporter/Cargo.toml --check
cargo clippy --manifest-path reporter/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path reporter/Cargo.toml --locked
docker compose config --quiet
docker compose build
```

The Rust tests use synthetic login URLs, a Unix-socket LocalAPI fixture, and an
HTTP Dappmanager fixture. They do not need real credentials or access to a tailnet.
Docker builds compile the reporter natively for the selected platform; the final
image contains only its static executable. CI builds both supported architectures.
An authenticated subnet/DNS check from another tailnet device remains a manual
integration test.
