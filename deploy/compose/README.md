# Castle on a single VPS — Compose + Caddy (no Kubernetes)

The lightweight deployment: one LUKS-encrypted VPS, rootless Podman (or Docker)
+ Compose, Caddy at the edge. Same containers and image as the k8s path — this
just drops the orchestrator. See `../k8s/` for the scale-up path.

## Why this exists

For a pilot (one client + a handful of decoys) Kubernetes is over-engineered:
its control-plane tax costs real money and real ops for orchestration you don't
yet need. This runs the identical castle image with a fraction of the machinery,
and makes provider-blindness (full-disk LUKS) simpler than the k8s CSI route.

## Topology

```
             Internet ──:443, per-codename pre-issued certs
                 │
            [ Caddy ]  edge · TLS termination · routing · access logs
                 │  auto_https OFF — certs are pre-issued in bulk, never on-demand
   ┌─────────────┼───────────────────────────────────────────┐
   │             │                                            │
 sso.<dom>   citadel.<dom>            ironwood.<dom> ...   (10 codenames)
   │             │                                            │
[Keycloak]  [oauth2-proxy-citadel] → castle → pg      [oauth2-proxy-ironwood]
   │         (real tenant, own realm+users)             → static://200
[kc-pg]                                                 (canary, empty realm)
```

Every codename subdomain looks identical from outside: same kind of cert, same
oauth2-proxy → Keycloak redirect. The only difference between a **canary** and a
**real tenant** is invisible externally:

| | oauth2-proxy upstream | Keycloak realm |
|---|---|---|
| canary  | `static://200` (no backend) | exists, **empty** (any login = attacker, logged) |
| tenant  | its own castle + Postgres   | populated with the client's users |

`promote` swaps a canary into a tenant: same subdomain, **same pre-issued cert**,
same realm name — so the conversion never touches the CA and leaves **no
Certificate Transparency trace**. An observer sees the whole pool issued once, in
bulk, and never learns which codenames became real or when.

## The certificate strategy (no wildcards)

Certs for the whole codename pool are issued **once, in one bulk batch**
(`castlectl issue-pool`), so CT shows them appearing together — no per-onboarding
timeline. Caddy runs with `auto_https off` and serves those static certs; it
never contacts the CA on its own. Renewal is also **batched** (renew the whole
pool together, well before the 90-day expiry) to keep CT showing synchronized
events, never a per-instance trickle. Individual certs (not a wildcard) mean one
leaked key never exposes the whole swarm.

Let's Encrypt caps 50 certs/registered-domain/week, which is why the pool is
sized ≤ 50 (10 is plenty for a pilot).

## Isolation

Each real tenant's `castle` + `postgres` sit on their **own network**, reachable
only by that tenant's oauth2-proxy. Nothing crosses between tenants. This is
shared-kernel container isolation — equivalent to the k8s NetworkPolicy model;
add gVisor/Kata or VM-per-tenant if a client's threat model demands a kernel
boundary.

## Layout

```
platform.compose.yml     Caddy + Keycloak + Keycloak's Postgres
templates/
  tenant.compose.yml      one real tenant: oauth2-proxy + castle + postgres
  canary.compose.yml      one canary: oauth2-proxy → static://200
  site.caddy              a codename's Caddy vhost (pre-issued cert)
caddy/
  Caddyfile               global config (auto_https off) + imports sites/*
  sites/                  generated per-codename vhosts + sso
  certs/                  pre-issued certs (git-ignored)
castlectl                 provisioner: issue-pool · allocate · promote · deprovision · list
.env.example             configuration
```

`keycloak-realm.sh` and `codenames.txt` are reused unchanged from `../k8s/`.

## Usage

```bash
cp .env.example .env && $EDITOR .env        # domain, Keycloak admin, etc.
docker compose -f platform.compose.yml up -d
./castlectl issue-pool 10                    # bulk-issue certs, stand up 10 canaries
./castlectl list
./castlectl promote citadel                  # turn a canary into a real tenant
./castlectl deprovision citadel              # back to a canary (cert kept)
```
