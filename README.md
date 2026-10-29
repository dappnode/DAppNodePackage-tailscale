# Tailscale for Dappnode

This package advertises the Dappnode networks `172.33.0.0/16` and `10.20.0.0/24`
and offers exit-node functionality. Approve the advertised routes and configure
DNS in the [Dappnode Tailscale setup guide](https://docs.dappnode.io/docs/user/access-your-dappnode/vpn/tailscale).

## Sign in and recover authentication

Provide `TS_AUTHKEY` in the setup wizard for automatic login, or leave it empty
and open the login URL in the package logs. Browser login has a one-hour startup
window (`TS_BOOT_TIMEOUT=1h`); authentication still requires your approval.
An unauthenticated package reports unhealthy while waiting for that approval.
If the window expires, the package restarts and generates another login request.

The `state` volume at `/var/lib/tailscale` retains the device identity.
`TS_AUTH_ONCE=true` reuses a valid saved login across restarts. Keep this volume
when updating or recovering the package.

If Tailscale explicitly rejects the configured auth key, startup retries once
with both `TS_AUTHKEY` and its upstream alias `TS_AUTH_KEY` removed from the
process environment. The package then waits for browser login instead of
repeatedly submitting the rejected credential. It keeps the same state volume,
routes, DNS preferences, and exit-node configuration. Other startup failures
retain their failure exit status.

Clear or replace a rejected key in **Tailscale package > Config** so it is also
removed from the saved configuration. The running container cannot edit that
configuration itself. To recover manually, clear the key, restart the package,
and open the fresh login URL from its logs. Treat login URLs as private.

## Networking and health

`TS_DEBUG_FIREWALL_MODE=auto` lets Tailscale select the host's available firewall
backend, including nftables. It is set in both the image and Compose defaults
so the image also works when an older configuration omits the variable.
The Compose sysctls enable IPv4 and IPv6 forwarding in the container's network
namespace; they do not change the host's global sysctls.

The image health check requires Tailscale to report `Running`, the local device
to be online, and no Tailscale health warnings. It surfaces authentication and
router/firewall errors even if the container remains running. This does not
prove that a remote client's ACLs, route approvals, or DNS configuration permit
access. After setup, also test a peer connection and access to a Dappnode service
from another tailnet device.

The container's Tailscale IP belongs to the VPN container. Host SSH is reached
through an approved subnet route to the Dappnode host, such as `10.20.0.1`.
Enabling Tailscale SSH inside this container does not provide a host shell.

## Development

Run `python3 -m unittest discover -s tests -v` with Python 3 and `jq` installed.
The tests use synthetic credentials and a mock containerboot; no tailnet login
is needed. The container smoke test runs a real unauthenticated daemon in an
isolated Docker network namespace:

```sh
upstream_version=$(jq -r .upstreamVersion dappnode_package.json)
docker build --build-arg "UPSTREAM_VERSION=$upstream_version" -t tailscale-package:test .
sh tests/container-smoke.sh tailscale-package:test
```

See [authenticated routing validation](tests/AUTHENTICATED-VALIDATION.md) for
manual subnet and exit-node results before and after a package restart.
