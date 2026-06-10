# syntax=docker/dockerfile:1
#
# The pinned fkst-substrate engine ref lives in backend/engine.ref (single
# line, 40-char commit SHA). Local build, from the repo root:
#
#   docker build -f backend/Dockerfile \
#     --build-arg FKST_SUBSTRATE_REF="$(cat backend/engine.ref)" \
#     -t fkst-hosted-api:dev .

ARG RUST_VERSION=1

# ---- engine build stage: compile fkst-substrate at a pinned ref ----
# The engine is encapsulated: built from a pinned upstream ref inside this image
# and never vendored into this repo. Only the compiled binaries and the two
# provenance files (ENGINE_SHA / ENGINE_VERSION) cross into the runtime stage.
FROM rust:${RUST_VERSION}-bookworm AS engine-builder
ARG FKST_SUBSTRATE_REF
ARG FKST_SUBSTRATE_REPO=https://github.com/ChronoAIProject/fkst-substrate.git
# Fail fast: pinning is mandatory. This guard runs before any clone.
RUN test -n "$FKST_SUBSTRATE_REF" || (echo "ERROR: FKST_SUBSTRATE_REF build-arg is required" && exit 1)
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /engine
# Uniform fetch of SHA / tag / branch; --depth 1 by-SHA works against GitHub
# (uploadpack.allowAnySHA1InWant). Fallback: if a server rejects an
# arbitrary-SHA shallow fetch, deepen to 50 commits of the default branch and
# check the ref out from there.
RUN git init -q \
    && git remote add origin "$FKST_SUBSTRATE_REPO" \
    && (git -c protocol.version=2 fetch --depth 1 origin "$FKST_SUBSTRATE_REF" \
        || (echo "INFO: shallow ref fetch failed; falling back to depth-50 default-branch fetch" \
            && git fetch --depth 50 origin)) \
    && (git checkout -q FETCH_HEAD 2>/dev/null || git checkout -q "$FKST_SUBSTRATE_REF") \
    && mkdir -p /engine-dist \
    && git rev-parse HEAD > /engine-dist/ENGINE_SHA \
    && echo "INFO: engine pinned at $(cat /engine-dist/ENGINE_SHA)"
# Build the engine workspace. Cache mounts speed up unchanged-ref rebuilds;
# binaries must be copied out inside the same RUN because the cache mounts
# are not part of the image filesystem.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/engine/target \
    cargo build --release --workspace --locked \
    && mkdir -p /engine-dist/bin \
    && cp target/release/fkst-framework /engine-dist/bin/fkst-framework \
    && if [ -f target/release/fkst-supervisor ]; then \
         cp target/release/fkst-supervisor /engine-dist/bin/fkst-supervisor; \
       else \
         echo "INFO: no fkst-supervisor in workspace"; \
       fi
RUN test -x /engine-dist/bin/fkst-framework \
    || (echo "ERROR: fkst-framework not produced by the engine build" && exit 1)
# Capture the engine version for the provenance file. Probe-confirmed: the
# version lives in the root Cargo.toml under [workspace.package] and is the
# first `version = "..."` line in that file.
RUN sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1 > /engine-dist/ENGINE_VERSION; \
    test -s /engine-dist/ENGINE_VERSION \
    || (echo "ERROR: could not extract engine version from the engine Cargo.toml" && exit 1); \
    echo "INFO: engine version $(cat /engine-dist/ENGINE_VERSION)"

# ---- server build stage ----
FROM rust:${RUST_VERSION}-bookworm AS server-builder
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
ARG FKST_SUBSTRATE_REF
ARG FKST_SUBSTRATE_REPO=https://github.com/ChronoAIProject/fkst-substrate.git
ARG PRODUCT_VERSION=0.0.0
# Engine runtime prerequisites: git + bash (spawned engine sessions use them),
# ca-certificates for outbound TLS.
RUN apt-get update && apt-get install -y --no-install-recommends git bash ca-certificates \
    && rm -rf /var/lib/apt/lists/*
RUN groupadd -g 10001 fkst && useradd -m -u 10001 -g 10001 -s /usr/sbin/nologin fkst
# Engine binaries: directory copy is the tolerant mechanism — fkst-framework is
# required (guarded in engine-builder); fkst-supervisor lands only when the
# pinned engine workspace builds it.
COPY --from=engine-builder /engine-dist/bin/ /usr/local/bin/
COPY --from=engine-builder /engine-dist/ENGINE_SHA /etc/fkst-engine-sha
COPY --from=engine-builder /engine-dist/ENGINE_VERSION /etc/fkst-engine-version
COPY --from=server-builder /app/target/release/fkst-hosted-api /usr/local/bin/fkst-hosted-api
ENV FKST_RUNTIME_ROOT=/var/lib/fkst/runtime \
    PATH=/usr/local/bin:$PATH \
    FKST_HOSTED_PORT=8080 \
    FKST_HOSTED_BIND_ADDR=0.0.0.0
RUN mkdir -p "$FKST_RUNTIME_ROOT" && chown -R fkst:fkst /var/lib/fkst
LABEL org.opencontainers.image.title="fkst-hosted-api" \
      org.opencontainers.image.version="${PRODUCT_VERSION}" \
      org.opencontainers.image.source="https://github.com/ChronoAIProject/fkst-hosted" \
      io.fkst.engine.repo="${FKST_SUBSTRATE_REPO}" \
      io.fkst.engine.ref="${FKST_SUBSTRATE_REF}"
# io.fkst.engine.sha / io.fkst.engine.version labels are applied by the CI
# docker-build workflow (and the documented local command) via `--label`,
# read back from /etc/fkst-engine-sha + /etc/fkst-engine-version, because
# LABEL cannot read a file. Those two files are the authoritative provenance.
VOLUME ["/var/lib/fkst/runtime"]
USER fkst
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/fkst-hosted-api"]
