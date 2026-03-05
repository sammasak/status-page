# Build stage - Alpine provides musl by default
FROM docker.io/rust:1.83-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml ./
COPY src ./src
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
RUN cargo build --release

# Runtime: scratch (fully static musl binary)
# Alpine rust builds static by default
FROM scratch
COPY --from=builder /build/target/release/status-page /status-page
EXPOSE 8080
ENTRYPOINT ["/status-page"]
