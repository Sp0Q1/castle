# Castle — production readiness

Status of getting Castle from "validated in a local lab" to "safe to hold real
clients' security findings." Legend: ✅ done · 🔶 partial · ⬜ not started.

## Foundations already built & validated (local kind + Calico)
- ✅ App: projects / findings / comments / roles, rich editor, uploads.
- ✅ Auth: oauth2-proxy + Keycloak (proxy mode), roles from groups, invite-gated
  onboarding, RP-initiated logout, mode-aware SPA.
- ✅ Multi-tenancy: per-tenant instance + DB, namespace + default-deny
  NetworkPolicy isolation (A/B proven), per-subdomain routing.
- ✅ Deception: decoy swarm — indistinguishable (proxy + canary realm) and
  credential-capturing (honeypot mode) decoys; `CanaryTouched` +
  `CanaryRealmLoginAttempt` alerts wired.
- ✅ Provisioner seed (`castlectl`), single-tenant Helm chart, image, local SSO test.

---

## Tier 1 — blockers before ANY real client data
- ⬜ **Secrets management.** The chart templates secrets from Helm values
  (plaintext). Move to sealed-secrets / external-secrets / Vault; real Keycloak
  client secrets + DB creds + cookie/JWT secrets out of `--set`.
- ⬜ **Database durability + backups.** Per-tenant Postgres is a single pod with
  no backups/HA/PITR. Add CloudNativePG (or scheduled `pg_dump` + WAL) and back
  up the uploads PVC. Restore drills.
- ⬜ **App security pass.** It's a security product: review the upload handler
  (type/size/anti-virus, path traversal), authz edges, rate limiting, security
  headers/CSP, injection, dependency audit.
- ⬜ **One real-cluster end-to-end run.** Everything so far is kind + sqlite +
  self-signed. Run on a real cluster with real Postgres, real Keycloak (prod
  mode + TLS), issued certs, and Kata/gVisor.
- 🔶 **Provisioning automation.** `castlectl` seeds it; prod needs per-tenant
  Keycloak realm creation + secret/codename mapping + (ideally) a Tenant CRD/operator.

## Tier 2 — hardening before scaling past a pilot
- ⬜ **Management plane.** Cross-client dashboard + client switcher behind VPN +
  mTLS (the crown jewels — highest-assurance surface). Undesigned/unbuilt.
- ⬜ **Backup/restore end-to-end** (Velero for namespaces+PVCs+secrets; keep the
  cert Secrets so restore doesn't re-issue and blow LE rate limits).
- ⬜ **Deploy the monitoring stack for real** (kube-prometheus-stack + Loki +
  Falco) and fire the alerts in anger; verify the feedback loop.
- ⬜ **Keycloak hardening.** Prod mode + TLS, `sslRequired`, no
  directAccessGrants/implicit, exact redirect + post-logout URIs, brute-force
  detection, verify-email + SMTP, strong admin, admin console not public.
- ⬜ **Certs at scale.** Past ~50/week move off LE HTTP-01 (commercial ACME or
  DNS-01); pre-issue the codename pool in bulk.
- ⬜ **SSL passthrough** (optional): terminate TLS in-pod so no central plaintext
  point (documented; deferred).
- ⬜ **Kata/gVisor** actually installed on nodes for kernel isolation.
- ⬜ **CI/CD**: build/test pipeline, image scanning, signed images, SBOM, pinned digests.

## Tier 3 — operational / compliance
- ⬜ Encryption at rest, data retention policy, immutable access audit, log
  retention, on-call/runbooks, and any DPAs/compliance for holding client vuln data.

---

## Recommended order to a pilot
1. Secrets management → 2. DB backups → 3. App security pass → 4. One real-cluster
run → 5. Keycloak hardening. That's the focused path to a **pilot** (one friendly
client, closely watched). The management plane + full scale-out follow.
