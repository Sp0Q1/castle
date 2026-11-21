# Castle on a single VPS — Compose + Caddy

One VPS, Podman (or Docker) Compose, Caddy at the edge. The picture of how the
pieces talk to each other, and what the design defends against, is in
[`docs/architecture.md`](../../docs/architecture.md); this page is about
running it.

## Why this exists

For a pilot (one client + a handful of decoys) Kubernetes is over-engineered:
its control-plane tax costs real money and real ops for orchestration you don't
yet need. This runs the same castle image with a fraction of the machinery.

## Topology

```
             Internet ──:443
                 │
            [ Caddy ]  edge · TLS termination · routing · access logs
                 │  pool certificates are issued in advance, in one batch
   ┌─────────────┼──────────────────────────────────────────┐
   │             │                                           │
 sso.<dom>   <codename>.<dom>                  <codename>.<dom> ...   (the pool)
   │             │                                           │
[Keycloak]  [oauth2-proxy] → castle → pg             [oauth2-proxy]
   │         (tenant, its realm has users)             → static://200
[kc-pg]                                               (canary, empty realm)
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

Which client a codename belongs to is recorded nowhere on the host. Keep that
mapping in your own store.

## The certificate strategy (no wildcards)

Certs for the whole codename pool are issued **once, in one bulk batch**
(`castlectl issue-certs N`), so CT shows them appearing together — no
per-onboarding timeline. Every pool vhost pins its pre-issued cert, so Caddy
never asks a CA for one. Renewal is also **batched** (`castlectl renew-pool` —
the whole pool together) to keep CT showing synchronized events, never a
per-instance trickle. Individual certs (not a wildcard) mean one leaked key
never exposes the whole swarm.

The sign-in host is the exception: it is always on and no decoy, so Caddy
obtains and renews that one certificate itself.

Issuance is done with **dehydrated** — a pure-bash ACME client, so no extra
binary beyond the `curl`/`openssl` castlectl already uses. Its model is a natural
fit: `castlectl` maintains a `domains.txt` (the pool) and a single `dehydrated -c`
run issues what's missing and renews what's due, all together. Your dehydrated
config owns the challenge (DNS-01 recommended for the pre-issue, since it needs
no port 80 or running server; HTTP-01 also works) and where certs land;
`castlectl` copies them into Caddy's cert dir. See `dehydrated/config.example`.
The lab path (`KC_INSECURE=1`) self-signs instead of touching a CA.

Let's Encrypt caps 50 certs/registered-domain/week, which is why the pool is
sized ≤ 50 (10 is plenty for a pilot).

## Isolation

Each real tenant's `castle` + `postgres` sit on their **own internal network**:
only that tenant's oauth2-proxy joins it, no other network is routed to it, and
the two reach nothing outside it. Castle also asks every request for the secret
it shares with that proxy, which `castlectl` generates per codename. Keycloak's
database is on an internal network of its own, with Keycloak alone. Nothing
crosses between tenants. This is shared-kernel container isolation — equivalent to the k8s NetworkPolicy model;
add gVisor/Kata or VM-per-tenant if a client's threat model demands a kernel
boundary.

## Layout

```
platform.compose.yml     Caddy + Keycloak + Keycloak's Postgres (+ BIND with --profile self-dns)
templates/
  tenant.compose.yml      one real tenant: oauth2-proxy + castle + postgres
  canary.compose.yml      one canary: oauth2-proxy → static://200
  site.caddy              a codename's Caddy vhost (pre-issued cert)
caddy/
  Caddyfile               global config + imports sites/*
  sites/                  generated per-codename vhosts + sso
  certs/                  pre-issued certs (git-ignored)
castlectl                 certificates, pool, promote/demote, upgrade, backup/restore, DNS
keycloak-realm.sh         creates a tenant's (or an empty canary's) Keycloak realm
codenames.txt             the pool every tenant and decoy is named from
branding/                 a file named `logo` here replaces the built-in brand (git-ignored)
dehydrated/               ACME client config and the DNS-01 hook
dns/                      the bundled nameserver's templates
systemd/                  the backup timer
docs/                     host setup, DNS, upgrades and backups, admin rotation
.env.example              configuration
```

## Host setup (once, before anything else)

A fresh VPS needs preparing first — packages, and two non-obvious bits that will
otherwise stop the deployment cold: use the **runc** runtime (crun + AppArmor
breaks Postgres), and free **port 53** if you self-host DNS. Full walkthrough with
the *why*: **`docs/host-setup.md`**. The one that bites hardest:

```bash
# podman must use runc, or Postgres crash-loops under AppArmor
mkdir -p /etc/containers/containers.conf.d
printf '[engine]\nruntime = "runc"\n' > /etc/containers/containers.conf.d/runtime.conf
```

## Bring-up

Host prepared (`docs/host-setup.md`) and, if self-hosting DNS, delegation done
(`docs/self-hosted-dns.md`). Then:

```bash
cp .env.example .env && $EDITOR .env                 # domain, Keycloak admin, etc.
cp dehydrated/config.example dehydrated/config && $EDITOR dehydrated/config
./castlectl issue-certs 10                            # one bulk cert batch for the pool
podman compose -f platform.compose.yml --profile self-dns up -d
./castlectl provision-pool                            # stand up the 10 canaries
./castlectl list
```

(`--profile self-dns` starts the bundled nameserver; omit it if you manage DNS
elsewhere. Use `docker compose` instead of `podman compose` if `ENGINE=docker`.)

Before a client's data arrives: replace the first-boot Keycloak admin
(`docs/admin-rotation.md`), set up backups and try a restore
(`docs/backup-and-upgrade.md`), and enable start on boot
(`docs/host-setup.md`).

## Day to day

| To | Run |
|----|-----|
| see every codename and whether it is a tenant, a canary or only has a cert | `./castlectl list` |
| take a client on | `./castlectl promote <codename>`, then add their people (below) |
| see who is in a codename's realm, and with which role | `./castlectl users <codename>` |
| move a tenant to another codename or host | `./castlectl export <codename>`, `./castlectl import <bundle> <codename>` (`docs/moving-and-retiring-a-tenant.md`) |
| end a tenant: export it, then destroy its data, people and secrets and leave a canary | `./castlectl demote <codename> --yes` |
| remove a canary and its realm, keeping the cert for reuse | `./castlectl deprovision <codename>` |
| add one more codename to the pool | `./castlectl allocate` |
| roll out a new castle image, proxy version or vhost template | `./castlectl upgrade` (`docs/backup-and-upgrade.md`) |
| back up, restore | `./castlectl backup`, `./castlectl restore <file> --yes` (`docs/backup-and-upgrade.md`) |
| renew the pool's certificates, all together | `./castlectl renew-pool` |
| put the DNS zone back in line with what runs | `./castlectl dns-sync` (`docs/self-hosted-dns.md`) |

**People and roles.** Accounts live in the codename's Keycloak realm, and the
group an account is in gives its role. A manager then puts people on projects
in the portal. Adding and removing people, sessions and what ends them are in
[`../oauth2-proxy-keycloak.md`](../oauth2-proxy-keycloak.md).

**Your own logo.** Put an image file named `logo` in `branding/` (PNG, JPEG,
GIF, WebP or SVG, at most 1 MiB) and run `./castlectl upgrade`. The file is
git-ignored and is not part of a backup bundle, so keep a copy.
