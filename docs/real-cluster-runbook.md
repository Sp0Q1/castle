# Castle — real-cluster deployment runbook

Everything up to this point has been validated on a kind lab (Calico, sqlite,
self-signed certs, hand-loaded images). This is the checklist to stand castle up
on a **real** cluster and provision the first tenant. It is the concrete form of
the one open Tier‑1 item, "one real-cluster end-to-end run".

Nothing here needs more application code — it is infrastructure standup. Where a
step could not be exercised in the lab, it says so.

> Assumes the `codename-only` castlectl (commands `allocate` / `provision`, no
> client name ever reaches the cluster).

---

## 0. What you must bring

| Thing | Why | Notes |
|---|---|---|
| A Kubernetes cluster | the target | managed (GKE/EKS/AKS) or self-managed |
| A **NetworkPolicy-enforcing CNI** | cross-tenant isolation is the core security property | Calico, Cilium — **not** the default in every managed cluster; verify it enforces |
| A domain | codenames need real DNS | e.g. `tenants.example.com`, `sso.example.com` |
| A container registry | pull the app image | ghcr is wired up by `release.yml`; any registry works |
| An ACME email | Let's Encrypt account | for cert-manager |

The isolation design **depends** on the CNI enforcing NetworkPolicy. kindnet did
not; we used Calico. On a managed cluster, confirm your CNI enforces before
trusting the default-deny (the lab's A/B test — reach a pod by IP, expect a
timeout, with an unrestricted control that succeeds — is the check).

---

## 1. Cluster prerequisites

Install the four platform components the chart assumes. Versions are examples;
pin what you test.

```bash
# ingress-nginx — the single, hardened TLS-termination point
helm upgrade --install ingress-nginx ingress-nginx \
  --repo https://kubernetes.github.io/ingress-nginx \
  --namespace ingress-nginx --create-namespace

# cert-manager — issues the per-codename certs
helm upgrade --install cert-manager cert-manager \
  --repo https://charts.jetstack.io --namespace cert-manager --create-namespace \
  --set crds.enabled=true

# Sealed Secrets — decrypts committed SealedSecrets in-cluster
helm upgrade --install sealed-secrets sealed-secrets \
  --repo https://bitnami-labs.github.io/sealed-secrets \
  --namespace kube-system

# (optional) monitoring — kube-prometheus-stack + loki, for the alert feedback loop
```

Then the castle platform layer (namespaces + the `letsencrypt-prod`
ClusterIssuer). **Edit `00-platform.yaml` first**: set the ACME email.

```bash
kubectl apply -f deploy/k8s/00-platform.yaml
kubectl label ns castle-system castle.io/managed=true   # so Keycloak's NetworkPolicy admits tenants
```

Verify the issuer is ready before continuing (a NotReady issuer means every
cert request silently pends):

```bash
kubectl get clusterissuer letsencrypt-prod -o wide
```

---

## 2. DNS

Point the codename hosts at the ingress controller's external address.

```bash
kubectl -n ingress-nginx get svc ingress-nginx-controller \
  -o jsonpath='{.status.loadBalancer.ingress[0].ip}'; echo
```

A **wildcard** `*.tenants.example.com` A record is simplest and — importantly —
leaks nothing: one record covers every codename, so DNS never reveals how many
tenants exist or when one appeared. Also add an A record for the IdP host
(`sso.example.com`).

Per-codename records work too, but pre-create them in bulk (like the certs) so a
new record never times-stamps an onboarding.

---

## 3. Publish the image

CI builds and scans the image on every PR but only **publishes** on push to
`main` and on version tags (`release.yml`). Cut a release:

```bash
git tag v0.1.0 && git push origin v0.1.0
```

This pushes `ghcr.io/<owner>/castle:0.1.0` (+ `:latest`, `:sha-<short>`), signs
it keyless with cosign, and attaches an SBOM. Pin deploys to the **digest**, and
optionally have the cluster require the signature (admission policy):

```bash
cosign verify ghcr.io/<owner>/castle@sha256:... \
  --certificate-identity-regexp '^https://github.com/<owner>/castle/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

If ghcr package is private, create a pull secret in each tenant namespace (or
make the package public for a pilot).

---

## 4. Keycloak (production)

The lab uses `local-keycloak.yaml` (start-dev / H2 / admin-admin) — never in
prod. Use `keycloak-prod.yaml` (start mode, Postgres, bootstrapped admin, TLS,
health probes). Set the hostname and provide the secret:

```bash
# 1. seal the admin + db credentials (never commit plaintext)
kubectl create secret generic keycloak-secrets -n castle-system \
  --from-literal=KC_DB_PASSWORD="$(openssl rand -base64 24)" \
  --from-literal=KC_BOOTSTRAP_ADMIN_USERNAME=admin \
  --from-literal=KC_BOOTSTRAP_ADMIN_PASSWORD="$(openssl rand -base64 24)" \
  --dry-run=client -o yaml | kubeseal --format yaml > keycloak-secrets-sealed.yaml
kubectl apply -f keycloak-secrets-sealed.yaml

# 2. set KC_HOSTNAME + the Ingress host in keycloak-prod.yaml (search "SET THIS")
kubectl apply -f deploy/k8s/keycloak-prod.yaml
kubectl -n castle-system rollout status deploy/keycloak
```

Then, **through the admin console once**: log in with the bootstrap admin,
create a named admin with MFA, disable the bootstrap account, and remove the two
`KC_BOOTSTRAP_*` env vars on the next rollout.

> Not yet boot-tested. `keycloak-prod.yaml` is schema-valid and derived from the
> lab-proven realm config (same client, mapper, hardening), but the `start`-mode
> + Postgres boot has not been run on a real cluster — validate it here before
> onboarding a client. The optimized-image hardening (`kc.sh build --db=postgres`
> then `start --optimized`) is a follow-up, not required for a pilot.

---

## 5. Provision the first tenant

castlectl in `prod` mode: creates the realm, generates one shared client secret,
seals the tenant's secrets, and installs the chart with `secrets.external=true`
and an internal Postgres.

```bash
export MODE=prod
export BASE_DOMAIN=tenants.example.com          # no EXTERNAL_PORT in prod (443)
export IMG_REPO=ghcr.io/<owner>/castle IMG_TAG=0.1.0
export KC_PUBLIC_URL=https://sso.example.com
export KC_INTERNAL=keycloak.castle-system.svc.cluster.local:8080
export KC_ADMIN_USER=<named-admin> KC_ADMIN_PASS=<...>   # KC_INSECURE unset — real cert

./deploy/k8s/castlectl.sh allocate            # -> prints a codename, e.g. "ironwood"
./deploy/k8s/castlectl.sh list
```

Record which client got that codename **in your own store, off this cluster** —
castlectl will not, by design (the cluster never learns a real client name).

Then in Keycloak, create the client's users in that realm (or wire an upstream
IdP / user federation) and add them to `castle-managers` / `castle-staff` /
`castle-clients`. First SSO login provisions them in the app at the matching
role.

> `prod`-mode sealing (`kubeseal` + the Sealed Secrets controller) is the one
> castlectl path not exercised in the lab — the lab has no controller. It
> delegates to the already-validated `seal-tenant.sh`; verify the sealed secret
> decrypts (`kubectl get secret <release>-app -n tenant-<codename>`) on the first
> real run.

---

## 6. The canary swarm

Provision decoys from the same pool so they are indistinguishable from real
tenants (same image, same SSO surface, an empty canary realm that records every
login attempt):

```bash
./deploy/k8s/castlectl.sh allocate --decoy      # repeat for a handful
```

Confirm a real tenant and a decoy are indistinguishable from outside (identical
302 → Keycloak, identical `mode`, identical headers) and that a login attempt on
a decoy realm produces a `LOGIN_ERROR` event (needs `eventsEnabled`, which the
generated realms set). Wire those events to the `CanaryRealmLoginAttempt` alert
once the monitoring stack is deployed.

---

## 7. Certificates at scale

The `letsencrypt-prod` issuer uses HTTP-01, ~50 certs/week — fine for the
initial phase (real tenants + canaries). **Issue the whole codename pool's certs
up front, in bulk**, not on the day each client signs: staggered issuance turns
the public CT log into an onboarding timeline, which is exactly the metadata the
codenames exist to hide. Past ~50/week, move to a commercial ACME or DNS-01.

---

## Post-deploy checklist

- [ ] CNI **enforces** NetworkPolicy (A/B isolation test passes)
- [ ] `letsencrypt-prod` ClusterIssuer Ready; a real cert issued (not the default)
- [ ] image pinned to a **digest**; cosign signature verifies
- [ ] Keycloak on Postgres, bootstrap admin replaced with an MFA admin
- [ ] a tenant's SealedSecret decrypts; the pod runs with `secrets.external=true`
- [ ] full SSO login works end to end (browser → Keycloak → app)
- [ ] real tenant vs decoy indistinguishable from outside
- [ ] backups running (`pg_dump` CronJob → S3) and a restore drill done
- [ ] monitoring stack deployed and an alert fired in anger

## Still open beyond a pilot

HA (single Keycloak + single Postgres today), PITR, the management plane
(cross-client dashboard behind mTLS), Kata/gVisor kernel isolation, and the
optimized Keycloak image. These are Tier‑2/3 in `production-readiness.md` and are
not blockers for a single, closely-watched pilot client.
