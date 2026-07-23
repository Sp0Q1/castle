# =============================================================================
# Optimized Keycloak image for castle's production IdP.
#
# Stock `quay.io/keycloak/keycloak` runs `kc.sh build` at container start, which
# writes the augmented Quarkus runner into /opt/keycloak/lib — so it cannot run
# on a read-only root filesystem. Here the build runs once, at IMAGE build time,
# baking in the build-time options (database vendor, health, metrics). The
# runtime container then does `start --optimized`, skips the boot-time build, and
# needs nothing writable under /opt/keycloak except an emptyDir at /opt/keycloak/data
# and /tmp. That is what lets keycloak-prod.yaml set readOnlyRootFilesystem: true.
#
# Published by .github/workflows/release.yml to
#   ghcr.io/<owner>/castle-keycloak:26.0
# (signed keyless with cosign, same as the app image). Pin deploys to the digest.
#
# Build-time options set below are FIXED for this image; changing the database
# vendor, health or metrics means rebuilding. Runtime options (hostname, DB URL
# and credentials, proxy headers) stay in the manifest's env, not here.
# =============================================================================
FROM quay.io/keycloak/keycloak:26.0 AS builder

# Build-time options: these are compiled into the optimized server and must NOT
# be repeated at runtime (with --optimized Keycloak ignores build options passed
# as runtime env, and warns about each one).
ENV KC_DB=postgres
ENV KC_HEALTH_ENABLED=true
ENV KC_METRICS_ENABLED=true

RUN /opt/keycloak/bin/kc.sh build

FROM quay.io/keycloak/keycloak:26.0
COPY --from=builder /opt/keycloak/ /opt/keycloak/
# The base image already runs as uid 1000; stated explicitly so it is verifiable
# from the Dockerfile itself and the container never falls back to root.
USER 1000
ENTRYPOINT ["/opt/keycloak/bin/kc.sh"]
