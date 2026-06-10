# syntax=docker/dockerfile:1
# ---- server build stage ----
FROM rust:1-bookworm AS server-builder
WORKDIR /app
# 1) cache the dependency graph first (manifests + lockfile + a stub main).
# The stub sources compile, so this warm build must succeed — no error masking.
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/fkst-hosted-api/Cargo.toml fkst-hosted-api/Cargo.toml
RUN mkdir -p fkst-hosted-api/src \
    && echo 'fn main() {}' > fkst-hosted-api/src/main.rs \
    && echo '' > fkst-hosted-api/src/lib.rs \
    && cargo build --release --locked --workspace
# 2) copy the real sources and build
# touch: the copied sources keep their (older) context mtimes, so without it
# cargo's mtime-based freshness check would keep the stub-built binary.
COPY backend/ ./
RUN touch fkst-hosted-api/src/main.rs fkst-hosted-api/src/lib.rs \
    && cargo build --release --locked --workspace
RUN test -x /app/target/release/fkst-hosted-api \
    || (echo "ERROR: fkst-hosted-api binary not produced by the server build" && exit 1)

# ---- runtime stage ----
FROM debian:bookworm-slim AS runtime
RUN useradd -r -u 10001 app
COPY --from=server-builder /app/target/release/fkst-hosted-api /usr/local/bin/fkst-hosted-api
USER 10001
ENV FKST_HOSTED_PORT=8080 \
    FKST_HOSTED_BIND_ADDR=0.0.0.0
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/fkst-hosted-api"]
