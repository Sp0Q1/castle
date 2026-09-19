# syntax=docker/dockerfile:1
# Multi-stage build for the castle image used by every tenant + the honeypot.
# The same image runs a real tenant (proxy auth, real DB) or a canary
# (fake DB, heavy logging) — only config/env differ, which keeps them
# fingerprint-identical from the outside.

# 1) Frontend (React SPA) -> frontend/dist, served by loco at runtime.
FROM node:26-slim@sha256:14bf3eac4bf209d906d3c41256597d3ab1f926b2e93a79e9bdfe1efd32454239 AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

# 2) Backend (Rust) -> release binary. Migrations are linked in via the
#    `migration` crate, so no migration files are needed at runtime.
FROM rust:1.98-slim-bookworm@sha256:ebd900bae66fd508b466cef82d64a83a5fb34682e4c8b2797a42908bddc95a57 AS backend
WORKDIR /app
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

# Dependencies are compiled in their own layer, against stub sources, so that
# layer is keyed on Cargo.lock alone. Copying src/ before the build (the obvious
# way to write this) means every one-line source edit recompiles all ~570
# dependencies from scratch — which is what made the CI image build take five
# minutes on every run, dwarfing the scan it exists to feed.
#
# The stubs must be real enough for cargo to resolve the workspace: the binary
# target and both lib targets have to exist, or cargo refuses before it gets as
# far as building dependencies.
COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml migration/Cargo.toml
RUN mkdir -p src/bin migration/src \
 && echo 'fn main() {}' > src/bin/main.rs \
 && : > src/lib.rs \
 && : > migration/src/lib.rs \
 && cargo build --release --bin castle-cli

# Now the real sources. Their own crates are rebuilt; the dependency layer above
# is reused untouched unless Cargo.lock changed.
COPY src/ src/
COPY migration/ migration/
# cargo decides staleness by mtime, and COPY preserves the source mtimes — which
# can be older than the stub artifacts just built. Without this the stub .rlib is
# considered current and the real code never gets compiled into the binary.
RUN touch src/lib.rs src/bin/main.rs migration/src/lib.rs \
 && cargo build --release --bin castle-cli

# 3) Runtime.
FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS runtime
# The digest above pins the *starting* layer for reproducibility, but Debian
# point releases (e.g. the libpcre2 fix in 10.42-1+deb12u1) land in the archive
# before the base image is rebuilt — so without an upgrade a known-fixed HIGH
# ships until the tag catches up, and CI's Trivy gate (rightly) goes red. Apply
# pending security updates at build time. Deliberate hadolint DL3005 waiver: the
# artifact is still deployed by immutable digest, and the Trivy gate is what
# actually guarantees no known HIGH/CRITICAL — not byte-identical rebuilds.
# hadolint ignore=DL3005
RUN apt-get update \
 && apt-get upgrade -y \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 castle
WORKDIR /app
COPY --from=backend /app/target/release/castle-cli /usr/local/bin/castle-cli
# Runtime needs: config (yaml read at boot), assets (static/i18n/mailer),
# and the built SPA (served from disk in production).
COPY config/ config/
COPY assets/ assets/
COPY --from=frontend /app/frontend/dist/ frontend/dist/
# Numeric UID (not the name) so Kubernetes runAsNonRoot can verify it.
USER 10001
EXPOSE 5150
ENTRYPOINT ["castle-cli"]
# Deployments override the args (binding/port/environment); this is a sane default.
CMD ["start", "--environment", "production"]
