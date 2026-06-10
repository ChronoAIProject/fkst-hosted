# syntax=docker/dockerfile:1
# ---- build stage ----
FROM rust:1-bookworm AS builder
WORKDIR /app
# 1) cache the dependency graph first (manifests + lockfile + a stub main)
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/fkst-hosted-api/Cargo.toml fkst-hosted-api/Cargo.toml
RUN mkdir -p fkst-hosted-api/src \
    && echo 'fn main() {}' > fkst-hosted-api/src/main.rs \
    && echo '' > fkst-hosted-api/src/lib.rs \
    && cargo build --release --locked --workspace || true
# 2) copy the real sources and build
# touch: the copied sources keep their (older) context mtimes, so without it
# cargo's mtime-based freshness check would keep the stub-built binary.
COPY backend/ ./
RUN touch fkst-hosted-api/src/main.rs fkst-hosted-api/src/lib.rs \
    && cargo build --release --locked --workspace

# ---- runtime stage ----
FROM debian:bookworm-slim AS runtime
RUN useradd -r -u 10001 app
COPY --from=builder /app/target/release/fkst-hosted-api /usr/local/bin/fkst-hosted-api
USER 10001
ENV FKST_HOSTED_PORT=8080 \
    FKST_HOSTED_BIND_ADDR=0.0.0.0
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/fkst-hosted-api"]
