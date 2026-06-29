# syntax=docker/dockerfile:1
#
# Slim controller image for the control-plane deployable (crate
# fkst-control-plane): the public REST API. It
# carries ONLY the fkst-control-plane binary + ca-certificates. The backend is a
# single crate now (fkst-shared + fkst-engine were folded in; the worker
# deployable was removed), so the dep-cache layer needs just the one manifest +
# Cargo.lock under backend/. The build context is the repo root.
#
# Local build, from the repo root:
#
#   docker build -f backend/fkst-control-plane/Dockerfile \
#     -t fkst-control-plane:dev .
#
# This slim image is API-only: it builds + runs (uid 10001) but carries no engine
# toolchain (codex/git/substrate), so it cannot run sessions. The pod-per-session
# execution image — control-plane + engine toolchain, run via a `run-session`
# subcommand — is built by milestone #9 (the run-session-image issue).

ARG RUST_VERSION=1
ARG PRODUCT_VERSION=0.0.0

# ---- server build stage: dep-cache warm-up, then build only the controller ----
FROM rust:${RUST_VERSION}-bookworm AS server-builder
WORKDIR /app
# 1) cache the dependency graph first (all crate manifests + lockfile +
#    stub sources). The stubs compile, so this warm build must succeed.
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/fkst-control-plane/Cargo.toml fkst-control-plane/Cargo.toml
RUN mkdir -p fkst-control-plane/src \
    && echo 'fn main() {}' > fkst-control-plane/src/main.rs \
    && echo '' > fkst-control-plane/src/lib.rs \
    && cargo build --release --locked --workspace
# 2) copy the real sources and build only the control-plane binary.
# touch: the copied sources keep their (older) context mtimes, so without it
# cargo's mtime-based freshness check would keep the stub-built binaries.
COPY backend/ ./
RUN touch fkst-control-plane/src/main.rs fkst-control-plane/src/lib.rs \
    && cargo build --release --locked -p fkst-control-plane
RUN test -x /app/target/release/fkst-control-plane \
    || (echo "ERROR: fkst-control-plane binary not produced by the server build" && exit 1)

# ---- control-plane runtime image (slim, engine-free) ----
# The controller deployable: public REST API only. No engine toolchain, no codex,
# no nyxid CLI, no runtime volume, no FKST_SUBSTRATE_REF. The pod-per-session
# execution image (milestone #9) is separate.
FROM debian:bookworm-slim AS control-plane-runtime
ARG PRODUCT_VERSION=0.0.0
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN groupadd -g 10001 fkst && useradd -m -u 10001 -g 10001 -s /usr/sbin/nologin fkst
COPY --from=server-builder /app/target/release/fkst-control-plane /usr/local/bin/fkst-control-plane
ENV FKST_ROLE=control-plane \
    PATH=/usr/local/bin:$PATH \
    FKST_HOSTED_PORT=8080 \
    FKST_HOSTED_BIND_ADDR=0.0.0.0
LABEL org.opencontainers.image.title="fkst-control-plane" \
      org.opencontainers.image.version="${PRODUCT_VERSION}" \
      org.opencontainers.image.source="https://github.com/ChronoAIProject/fkst-hosted"
USER fkst
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/fkst-control-plane"]
