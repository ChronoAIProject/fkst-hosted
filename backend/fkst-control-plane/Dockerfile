# syntax=docker/dockerfile:1
#
# Slim controller image for the control-plane deployable (crate
# fkst-control-plane): the public REST API + claim/placement authority. It
# carries ONLY the fkst-control-plane binary + ca-certificates — no engine, no
# codex, no nyxid CLI, and no runtime volume. The build context is the repo root
# (the dep-cache layering needs every workspace crate manifest + Cargo.lock,
# which live under backend/).
#
# Local build, from the repo root:
#
#   docker build -f backend/fkst-control-plane/Dockerfile \
#     -t fkst-control-plane:dev .
#
# TRANSITIONAL (until #151): engine execution still lives in the control-plane
# crate today, so the control-plane process still spawns engine children at
# runtime. This slim image therefore BUILDS and runs (uid 10001) but cannot yet
# run engine sessions; until #151 moves engine execution onto the worker, the
# control-plane Deployment runs from the engine-laden worker image (see
# backend/fkst-worker/Dockerfile and backend/deploy/k8s/README.md). The slim
# image becomes the controller's production home once #151 lands.

ARG RUST_VERSION=1
ARG PRODUCT_VERSION=0.0.0

# ---- server build stage: dep-cache warm-up, then build only the controller ----
FROM rust:${RUST_VERSION}-bookworm AS server-builder
WORKDIR /app
# 1) cache the dependency graph first (all crate manifests + lockfile +
#    stub sources). The stubs compile, so this warm build must succeed.
COPY backend/Cargo.toml backend/Cargo.lock ./
COPY backend/fkst-shared/Cargo.toml fkst-shared/Cargo.toml
COPY backend/fkst-engine/Cargo.toml fkst-engine/Cargo.toml
COPY backend/fkst-journal/Cargo.toml fkst-journal/Cargo.toml
COPY backend/fkst-control-plane/Cargo.toml fkst-control-plane/Cargo.toml
COPY backend/fkst-worker/Cargo.toml fkst-worker/Cargo.toml
RUN mkdir -p fkst-shared/src fkst-engine/src fkst-journal/src fkst-control-plane/src fkst-worker/src \
    && echo '' > fkst-shared/src/lib.rs \
    && echo '' > fkst-engine/src/lib.rs \
    && echo '' > fkst-journal/src/lib.rs \
    && echo 'fn main() {}' > fkst-control-plane/src/main.rs \
    && echo '' > fkst-control-plane/src/lib.rs \
    && echo 'fn main() {}' > fkst-worker/src/main.rs \
    && echo '' > fkst-worker/src/lib.rs \
    && cargo build --release --locked --workspace
# 2) copy the real sources and build only the control-plane binary.
# touch: the copied sources keep their (older) context mtimes, so without it
# cargo's mtime-based freshness check would keep the stub-built binaries.
COPY backend/ ./
RUN touch fkst-shared/src/lib.rs fkst-engine/src/lib.rs fkst-journal/src/lib.rs \
        fkst-control-plane/src/main.rs fkst-control-plane/src/lib.rs \
        fkst-worker/src/main.rs fkst-worker/src/lib.rs \
    && cargo build --release --locked -p fkst-control-plane
RUN test -x /app/target/release/fkst-control-plane \
    || (echo "ERROR: fkst-control-plane binary not produced by the server build" && exit 1)

# ---- control-plane runtime image (slim, engine-free) ----
# The controller deployable: public REST API + claim/placement authority. No
# engine, no codex, no nyxid CLI, no runtime volume, no FKST_SUBSTRATE_REF.
# TRANSITIONAL: until #151 moves engine execution onto the worker, the
# control-plane process still spawns engine children, so this slim image is not
# yet the controller's production home (the worker image is — see header).
# This image exists so the structure + slimming are in place for #151.
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
