# syntax=docker/dockerfile:1
# Multi-stage build for the castle image used by every tenant + the honeypot.
# The same image runs a real tenant (proxy auth, real DB) or a canary
# (fake DB, heavy logging) — only config/env differ, which keeps them
# fingerprint-identical from the outside.

# 1) Frontend (React SPA) -> frontend/dist, served by loco at runtime.
FROM node:22-slim AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY frontend/ ./
RUN npm run build

# 2) Backend (Rust) -> release binary. Migrations are linked in via the
#    `migration` crate, so no migration files are needed at runtime.
FROM rust:1.97-slim-bookworm AS backend
WORKDIR /app
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY migration/ migration/
COPY src/ src/
RUN cargo build --release --bin castle-cli

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
