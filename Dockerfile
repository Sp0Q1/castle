# syntax=docker/dockerfile:1
# Multi-stage build for the castle image used by every tenant + the honeypot.
# The same image runs a real tenant (proxy auth, real DB) or a canary
# (fake DB, heavy logging) — only config/env differ, which keeps them
# fingerprint-identical from the outside.

# 1) Frontend (React SPA) -> frontend/dist, served by loco at runtime.
FROM node:26-slim AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

# 2) Backend (Rust) -> release binary. Migrations are linked in via the
#    `migration` crate, so no migration files are needed at runtime.
FROM rust:1.96-slim-bookworm AS backend
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
# The stubs must be real enough for cargo to resolve the workspace: both binary
# targets and both lib targets have to exist, or cargo refuses before it gets as
# far as building dependencies.
COPY Cargo.toml Cargo.lock ./
COPY migration/Cargo.toml migration/Cargo.toml
RUN mkdir -p src/bin migration/src \
 && echo 'fn main() {}' > src/bin/main.rs \
 && echo 'fn main() {}' > src/bin/tool.rs \
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
RUN touch src/lib.rs src/bin/main.rs src/bin/tool.rs migration/src/lib.rs \
 && cargo build --release --bin castle-cli

# 3) Runtime.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
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
