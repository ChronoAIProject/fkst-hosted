# syntax=docker/dockerfile:1
#
# The pinned fkst-substrate engine ref lives in backend/engine.ref (single
# line, 40-char commit SHA). Local build, from the repo root:
#
#   docker build -f backend/Dockerfile \
#     --build-arg FKST_SUBSTRATE_REF="$(cat backend/engine.ref)" \
#     -t fkst-hosted-api:dev .

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
# loudly rather than producing an image without codex.
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
# v0.7.0 — the CLI auth contract is version-specific.
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
COPY --from=server-builder /app/target/release/fkst-hosted-api /usr/local/bin/fkst-hosted-api
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
