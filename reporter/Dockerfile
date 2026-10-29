FROM rust:1.99.0-alpine3.23 AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM scratch
COPY --from=build /build/target/release/tailscale-status-reporter /tailscale-status-reporter
ENTRYPOINT ["/tailscale-status-reporter"]
