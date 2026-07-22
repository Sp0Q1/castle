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
- ✅ **Secrets management.** Chart gained `secrets.external: true` — in prod it
  creates no Secret; the `<release>-app`/`-db` Secrets are provided as
  **SealedSecrets** (encrypted, committable; controller decrypts in-cluster). See
  `deploy/k8s/secrets/` (`seal-tenant.sh` + README). Validated on kind end to
  end: a tenant ran from a sealed secret with zero plaintext in Helm. (Backup the
  controller's sealing key; ESO is a drop-in alternative via the same contract.)
- 🔶 **Database durability + backups.** Scheduled `pg_dump` (+ uploads) → S3
  CronJob (`backup.enabled`), with a backup-egress NetworkPolicy. Validated on
  kind end to end: hourly dump → MinIO → restore into a fresh DB recovered the
  data. Fixed two bugs found here: `connect_timeout` 500ms→5000ms (Postgres
  tenants were crash-looping) and mc config leaking S3 creds into the bucket.
  Still open: **HA** (single Postgres pod) and **PITR** — documented CloudNativePG
  as the upgrade when RPO/uptime demand it. Retention = bucket lifecycle policy.
- ✅ **App security pass.** Adversarial multi-agent review (6 dimensions, 20 raw
  findings → 9 confirmed). **All 9 fixed:**
  - Uploads: type now from **magic bytes** (SVG rejected outright — it could carry
    script); serving is auth-gated and sends `nosniff` + CSP + `Content-Disposition`.
  - Frontend: mermaid `securityLevel: strict`; **rehype-sanitize** appended after
    `rehype-raw` (raw-HTML stored XSS in findings/comments); token moved to
    per-tab `sessionStorage`.
  - Config: `secure_headers` enabled with a CSP (script-src without
    `unsafe-inline`); JWT secret has **no default** (fails closed).
  - Authz: SSO demotion now downgrades stale `project_members` rows.
  Runtime-verified: upload validation (incl. SVG renamed `.png` → rejected), the
  401 auth gate, response headers, and fail-closed startup. The three frontend
  fixes + membership downgrade are compile-verified; confirm in a browser on the
  next lab run.
- ✅ **Rate limiting.** Two layers: per-account throttling in `src/rate_limit.rs`
  (login 8/15min, register/forgot/magic-link 5/hr, keyed on the submitted email
  and checked *before* the user lookup so a guess never costs an argon2 hash),
  and per-source-address `limit-rps`/`limit-connections` at the ingress via the
  chart's `rateLimit` values. The decoys apply identical limits and return the
  same 429, so they cannot be told apart from production by hammering them —
  capture happens before the check, so throttled attempts are still recorded.
  In-pod state is process-local, which is exact at one replica per tenant and
  documented as needing a shared store if that ever changes.
- ✅ **Automated tests.** `tests/requests/authz.rs` covers the access rules over
  the real HTTP surface — draft invisibility to clients (404, not 401),
  non-member rejection, client write/publish refusal, the upload auth gate,
  field limits, payload round-tripping, and login throttling. Verified they can
  fail by reverting each rule in turn. Plus unit tests for validation and the
  rate limiter. Still thin on: SSO provisioning/demotion (`provision_from_sso`)
  and the proxy-mode header trust path, both of which need proxy-mode fixtures.
- ✅ **Input validation.** `src/validation.rs` bounds every user-supplied text
  field and rejects NUL/control characters (which Postgres refuses in `text`
  columns — a 500, not a 400). Deliberately validation and *not* sanitization:
  findings legitimately quote exploit payloads, so content round-trips verbatim
  and XSS defense stays at render time. Anything that renders markdown outside
  the browser — a future **PDF/HTML export or HTML mail** — must add its own
  output sanitization (`ammonia`), because the frontend's does not cover it.
  Also fixed a contradiction: loco's default 2MB body limit would have rejected
  image uploads under the 10 MiB cap in `uploads.rs`; raised to 12mb.
  Runtime-verified: 10/10 cases, including a payload stored byte-identical.
- ⬜ **One real-cluster end-to-end run.** Everything so far is kind + sqlite +
  self-signed. Run on a real cluster with real Postgres, real Keycloak (prod
  mode + TLS), issued certs, and Kata/gVisor.
- ✅ **Provisioning automation.** `castlectl onboard "<client>"` is one command:
  allocate a random codename from a pre-issued pool, create a hardened
  per-tenant Keycloak realm (`keycloak-realm.sh`), generate one client secret
  shared by the realm and oauth2-proxy, install the chart, and record the
  codename→client mapping in a control-plane Secret. `proxy`/`prod`/`local`
  modes; `deprovision` tears down namespace + realm + mapping. Lab-verified end
  to end incl. a full SSO login flow (proving the shared secret matches).
  Remaining: prod-mode sealing is unexercised (no kubeseal/SealedSecrets
  controller in the lab; delegates to the validated `seal-tenant.sh`), and a
  Tenant CRD/operator is still the eventual goal over the shell wrapper.

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
- 🔶 **CI/CD.** Built (`.github/workflows/`): `ci.yml` runs fmt / clippy
  `-D warnings` / tests, biome + tsc + frontend build, `helm lint` plus a render
  of every deployment mode validated against real Kubernetes schemas
  (kubeconform), and actionlint. `security.yml` runs TruffleHog over full git
  history, Semgrep OSS, `cargo audit`, `npm audit` and Trivy (image CVEs block;
  k8s/Dockerfile misconfiguration reports to the Security tab without blocking),
  weekly as well as on push. Dependabot covers cargo/npm/actions/docker with a
  7-day cooldown; every action is SHA-pinned. Accepted `cargo audit` advisories
  are documented one-by-one in `.cargo/audit.toml` with the condition that
  should remove each.
  Still open: **signed images (cosign), SBOM (syft//attestation), digest-pinned
  base images**, and pushing the image from CI to a registry at all.

## Tier 3 — operational / compliance
- ⬜ Encryption at rest, data retention policy, immutable access audit, log
  retention, on-call/runbooks, and any DPAs/compliance for holding client vuln data.

---

## Recommended order to a pilot
1. Secrets management → 2. DB backups → 3. App security pass → 4. One real-cluster
run → 5. Keycloak hardening. That's the focused path to a **pilot** (one friendly
client, closely watched). The management plane + full scale-out follow.
