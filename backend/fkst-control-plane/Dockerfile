# syntax=docker/dockerfile:1
#
# Session-capable image for the single fkst-control-plane crate. The SAME binary
# is the API server (no args) and, inside a pod-per-session Job, the engine
# runner (`run-session`, milestone #9) — so this image bundles the engine
# toolchain: the codex CLI, the fkst-substrate engine (built at the pinned
# engine ref), the nyxid CLI, and git + the runtime volume. The build context is
# the repo root.
#
# Local build, from the repo root (the engine ref is defaulted, #227):
#
#   docker build -f backend/fkst-control-plane/Dockerfile -t fkst-control-plane:dev .
#
# The fkst-substrate engine is built from a pinned upstream ref
# (backend/engine.ref); keep the ARG defaults below in lockstep with it.

ARG RUST_VERSION=1
# Operator-pinned codex CLI version (issue #112). A Feb-2026+ release: the
# engine runs `codex exec --dangerously-bypass-approvals-and-sandbox` and the
# default chrono-llm provider needs `wire_api = "responses"`, so an older codex
# breaks the run. Bumped deliberately, never floated to "latest".
ARG CODEX_VERSION=0.139.0

# ---- codex install stage: download the prebuilt release asset ----
# This is a DOWNLOAD of a published binary, not a cargo build — no registry/
# target cache mount, no build-race risk. The musl asset is static, so it runs
# as-is in the debian-slim runtime stage. TARGETARCH maps onto the codex asset
# arch token (amd64 -> x86_64, arm64 -> aarch64); any other arch fails the build
# loudly rather than producing an image without codex. Feeds the WORKER image.
FROM debian:bookworm-slim AS codex-builder
ARG CODEX_VERSION
ARG TARGETARCH
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates tar \
    && rm -rf /var/lib/apt/lists/*
RUN set -eux; \
    case "${TARGETARCH:-amd64}" in \
      amd64) A=x86_64;; \
      arm64) A=aarch64;; \
      *) echo "ERROR: unsupported TARGETARCH '${TARGETARCH}' for codex install" && exit 1;; \
    esac; \
    curl -fsSL -o /tmp/codex.tgz \
      "https://github.com/openai/codex/releases/download/rust-v${CODEX_VERSION}/codex-${A}-unknown-linux-musl.tar.gz"; \
    tar xzf /tmp/codex.tgz -C /tmp; \
    mv "/tmp/codex-${A}-unknown-linux-musl" /usr/local/bin/codex; \
    chmod +x /usr/local/bin/codex; \
    rm -f /tmp/codex.tgz

# ---- engine build stage: compile fkst-substrate at a pinned ref ----
# The engine is encapsulated: built from a pinned upstream ref inside this image
# and never vendored into this repo. Only the compiled binaries and the two
# provenance files (ENGINE_SHA / ENGINE_VERSION) cross into the WORKER runtime.
FROM rust:${RUST_VERSION}-bookworm AS engine-builder
# Defaulted to the pinned SHA as a temporary workaround (#227): the current build
# platform can't pass build-time args. Keep in lockstep with backend/engine.ref.
ARG FKST_SUBSTRATE_REF=cb072b224d817a8a4b57f92b613cf5f2174d8759
ARG FKST_SUBSTRATE_REPO=https://github.com/ChronoAIProject/fkst-substrate.git
# Fail fast on an explicitly empty pin. This guard runs before any clone; the
# default above pins a no-arg build, but `--build-arg FKST_SUBSTRATE_REF=""`
# still fails here (the CI negative test depends on this behaviour).
RUN test -n "$FKST_SUBSTRATE_REF" || (echo "ERROR: FKST_SUBSTRATE_REF build-arg is required" && exit 1)
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /engine
# Uniform fetch of SHA / tag / branch; --depth 1 by-SHA works against GitHub
# (uploadpack.allowAnySHA1InWant). Fallback: if a server rejects an
# arbitrary-SHA shallow fetch, deepen to 50 commits of the default branch and
# check out the pinned ref from there. Each fetch path checks out its own
# target — FETCH_HEAD only on the primary path, where it IS the requested
# ref — so the fallback can never silently land on a default-branch tip.
# When the ref is a full commit SHA, a hard assertion makes any drift fail
# the build instead of producing a green-but-unpinned engine.
RUN git init -q \
    && git remote add origin "$FKST_SUBSTRATE_REPO" \
    && ((git -c protocol.version=2 fetch --depth 1 origin "$FKST_SUBSTRATE_REF" \
         && git checkout -q FETCH_HEAD) \
        || (echo "INFO: shallow ref fetch failed; falling back to depth-50 default-branch fetch" \
            && git fetch --depth 50 origin \
            && git checkout -q "$FKST_SUBSTRATE_REF")) \
    && if echo "$FKST_SUBSTRATE_REF" | grep -Eq '^[0-9a-f]{40}$'; then \
         [ "$(git rev-parse HEAD)" = "$FKST_SUBSTRATE_REF" ] \
         || (echo "ERROR: engine HEAD $(git rev-parse HEAD) != pinned ref $FKST_SUBSTRATE_REF" && exit 1); \
       fi \
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
# Capture the engine version for the provenance file. The parse is anchored
# to the [workspace.package] table of the engine's root Cargo.toml — the
# scan tracks the current TOML table, so a `version` key in any other table
# (e.g. a dependency spec) can never be picked up — and the extracted value
# must be semver-shaped.
RUN awk -F'"' '/^\[/ { table = $0 } table == "[workspace.package]" && /^version[ \t]*=/ { print $2; exit }' Cargo.toml \
        > /engine-dist/ENGINE_VERSION; \
    test -s /engine-dist/ENGINE_VERSION \
    || (echo "ERROR: no version key found under [workspace.package] in the engine Cargo.toml" && exit 1); \
    grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+' /engine-dist/ENGINE_VERSION \
    || (echo "ERROR: engine version '$(cat /engine-dist/ENGINE_VERSION)' is not semver-shaped" && exit 1); \
    echo "INFO: engine version $(cat /engine-dist/ENGINE_VERSION)"

# ---- nyxid CLI build stage: compile the nyxid CLI at a pinned tag ----
# A fkst-package's business logic may shell out to `nyxid ...` to reach a
# NyxID-brokered service AS the session user (#103). The per-session
# NYXID_ACCESS_TOKEN + NYXID_URL it authenticates with are provisioned by #111;
# this stage only puts the binary on PATH and proves it launches. Pinned to
# v0.7.0 — the CLI auth contract is version-specific. Feeds the WORKER image.
ARG NYXID_CLI_REF=v0.7.0
FROM rust:${RUST_VERSION}-bookworm AS cli-builder
ARG NYXID_CLI_REF
ARG NYXID_REPO=https://github.com/ChronoAIProject/NyxID.git
# git+ca-certificates for the clone; libdbus-1-dev + pkg-config because the CLI
# pulls libdbus-sys (its OS-keyring credential store), whose build.rs links the
# system libdbus via pkg-config. The runtime stage carries the matching
# libdbus-1-3 so the binary loads (the pod uses NYXID_ACCESS_TOKEN, not the
# keyring, but the binary still dynamically links libdbus).
RUN apt-get update && apt-get install -y --no-install-recommends \
        git ca-certificates libdbus-1-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /nyxid
# Same uniform shallow-by-ref fetch with a depth-50 fallback as engine-builder.
# The crate is the workspace member `cli` (crate `nyxid-cli`, bin `nyxid`).
RUN git init -q \
    && git remote add origin "$NYXID_REPO" \
    && ((git -c protocol.version=2 fetch --depth 1 origin "$NYXID_CLI_REF" \
         && git checkout -q FETCH_HEAD) \
        || (echo "INFO: shallow ref fetch failed; falling back to depth-50 fetch" \
            && git fetch --depth 50 origin \
            && git checkout -q "$NYXID_CLI_REF"))
# Distinct cache-mount ids: the engine-builder also mounts a cache at
# /usr/local/cargo/registry (default id = the path), and BuildKit builds the
# two stages in PARALLEL — sharing that registry cache races on .cargo-ok
# (`File exists (os error 17)`). Isolating the ids gives this stage its own
# registry/target caches.
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=nyxid-cli-cargo-registry \
    --mount=type=cache,target=/nyxid/target,id=nyxid-cli-cargo-target \
    cargo build --release --locked -p nyxid-cli \
    && mkdir -p /cli-dist \
    && cp target/release/nyxid /cli-dist/nyxid
RUN test -x /cli-dist/nyxid \
    || (echo "ERROR: nyxid binary not produced by the CLI build" && exit 1)

# ---- server build stage: dep-cache warm-up, then build only the control-plane ----
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

# ---- worker runtime image (engine-laden) ----
# The WORKER deployable. It carries the engine binaries + codex + nyxid CLI and
# the runtime volume. (The transitional control-plane binary copy is dropped:
# the controller now has its own image — backend/fkst-control-plane/Dockerfile.
# Until #151, the control-plane Deployment runs from this image via a `command:`
# override; that arrangement is documented in the README, not encoded by
# carrying the controller binary here — see header.)
FROM debian:bookworm-slim AS control-plane-runtime
# Same default as the engine-builder stage (#227); feeds the io.fkst.engine.ref
# label below. Keep in lockstep with backend/engine.ref.
ARG FKST_SUBSTRATE_REF=cb072b224d817a8a4b57f92b613cf5f2174d8759
ARG FKST_SUBSTRATE_REPO=https://github.com/ChronoAIProject/fkst-substrate.git
ARG PRODUCT_VERSION=0.0.0
ARG CODEX_VERSION
# git + bash for spawned engine sessions, ca-certificates for outbound TLS
# (codex talks HTTPS to its LLM provider), and libdbus-1-3 so the bundled nyxid
# CLI (which links libdbus via its keyring store) loads in the slim runtime.
RUN apt-get update && apt-get install -y --no-install-recommends \
        git bash ca-certificates libdbus-1-3 \
    && rm -rf /var/lib/apt/lists/*
RUN groupadd -g 10001 fkst && useradd -m -u 10001 -g 10001 -s /usr/sbin/nologin fkst
# Engine binaries: directory copy is the tolerant mechanism — fkst-framework is
# required (guarded in engine-builder); fkst-supervisor lands only when the
# pinned engine workspace builds it.
COPY --from=engine-builder /engine-dist/bin/ /usr/local/bin/
COPY --from=engine-builder /engine-dist/ENGINE_SHA /etc/fkst-engine-sha
COPY --from=engine-builder /engine-dist/ENGINE_VERSION /etc/fkst-engine-version
COPY --from=server-builder /app/target/release/fkst-control-plane /usr/local/bin/fkst-control-plane
# nyxid CLI for in-session NyxID-brokered business calls (#103). Assert it
# launches in the slim runtime (catches a missing shared lib) and reports the
# pinned v0.7.0.
COPY --from=cli-builder /cli-dist/nyxid /usr/local/bin/nyxid
RUN nyxid --version && nyxid --version | grep -q "0.7.0"
# codex CLI (issue #112): the engine requires it on PATH (fkst-substrate does
# not install it). The static musl binary needs no extra runtime libs; the
# assertion below fails the build on version drift OR a missing library (a
# launch failure would make `codex --version` exit non-zero).
COPY --from=codex-builder /usr/local/bin/codex /usr/local/bin/codex
RUN test "$(codex --version | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)" = "$CODEX_VERSION" \
    && codex --version
ENV FKST_ROLE=control-plane \
    FKST_RUNTIME_ROOT=/var/lib/fkst/runtime \
    PATH=/usr/local/bin:$PATH \
    FKST_HOSTED_PORT=8080 \
    FKST_HOSTED_BIND_ADDR=0.0.0.0
RUN mkdir -p "$FKST_RUNTIME_ROOT" && chown -R fkst:fkst /var/lib/fkst
LABEL org.opencontainers.image.title="fkst-control-plane" \
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
ENTRYPOINT ["/usr/local/bin/fkst-control-plane"]
