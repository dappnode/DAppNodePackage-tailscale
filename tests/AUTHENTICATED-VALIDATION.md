# Authenticated routing validation

Date: 23 October 2026. Implementation commit: `91e2f90d1c34b0414d3efd63785e92ae085c0c6f`.

The PR image was manually tested on `linux/amd64`, using Tailscale **1.104.2**
and the existing authenticated state volume. Requests originated from a separate
authenticated Linux computer. The phone was not used.

| Check | Original baseline | PR image | PR image after restart | Original image restored |
| --- | --- | --- | --- | --- |
| HTTP challenge through `10.20.0.0/24` | Pass | Pass | Pass | Pass |
| HTTP challenge through `172.33.0.0/16` | Pass | Pass | Pass | Pass |
| IPv4 internet egress through the exit node | Pass | Pass | Pass | Pass |
| Authenticated and online, with no Tailscale health warnings | Pass | Pass | Pass | Pass |

The client opened a temporary challenge page on the Dappnode. It fetched
independent endpoints on both private networks and an external HTTPS public-IP
endpoint. The private endpoints observed the router's expected source-NAT
addresses. The external endpoint returned the Dappnode's public IPv4 address.
Packet observation on `tailscale0`, restricted to this client's test traffic,
also confirmed packets to both private endpoints and the external HTTPS
endpoint. The egress result therefore includes packet-path evidence in addition
to matching public IP addresses.

The PR image's Docker health check reported **healthy**. It preserved the
Tailscale addresses, reconnected from saved authentication after restart, and
had zero automatic restarts. The installed entrypoint and health-check hashes
matched the repository scripts.

The original image was restored after testing. Persistent Compose settings,
the named state volume, network addresses, and other service containers were
preserved. Connectivity was verified again after restoration. Temporary probe
listeners and packet observers were then removed.

The client briefly disabled exit-node use during the first PR-image probe,
which produced a nonmatching public IP. After re-enabling it, the PR-image,
restart, and restoration checks all passed. This client-setting interruption
was not counted as a package failure.

## Limits

- This is a manual authenticated test; CI does not reproduce these steps.
- ARM64 execution and IPv6 internet egress were not tested.
- HTTP subnet routing and HTTPS IPv4 egress were exercised; this does not prove
  every application, protocol, DNS setup, or tailnet ACL configuration works.
- No real credentials, public IP addresses, or unrelated packet payloads are
  included in this report.
