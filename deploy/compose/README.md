# Castle on a single VPS — Compose + Caddy

One VPS, Podman (or Docker) Compose, Caddy at the edge. The picture of how the
pieces talk to each other, and what the design defends against, is in
[`docs/architecture.md`](../../docs/architecture.md); this page is about
running it.

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
the whole pool together), so CT shows the pool moving as one. Individual certs
(not a wildcard) mean one leaked key never exposes the whole swarm.
`castlectl allocate` is the exception: it adds a single codename and issues its
certificate alone, which CT shows as one event of its own. Grow the pool with
`issue-certs` when that matters.

Nothing renews the pool unless it is asked to. `systemd/castle-renew.timer`
asks once a week: it runs `renew-pool`, and dehydrated renews every certificate
that has fewer days left than its `RENEW_DAYS` (32, unless `dehydrated/config`
says otherwise). Caddy is restarted only when a certificate changed. A name
that fails does not hold up the others: what was renewed is served, and the run
ends non-zero. Enable the timer once:

```bash
# paths in the unit assume /opt/castle — edit them if you cloned elsewhere
cp systemd/castle-renew.{service,timer} /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now castle-renew.timer
```

A run that failed shows in `systemctl --failed` and in
`journalctl -u castle-renew.service`. With the bundled nameserver a renewal
needs that one running.

The sign-in host is the exception: it is always on and no decoy, so Caddy
obtains and renews that one certificate itself.

Issuance is done with **dehydrated**, an ACME client written in shell (a
package of its own: `docs/host-setup.md`). `castlectl` maintains a
`domains.txt` (the pool), and a single `dehydrated -c` run issues what is
missing and renews what is due, all together. `dehydrated/config` says how the
challenge is answered and where certificates land, and `castlectl` copies them
to where Caddy reads them. The example config answers DNS-01 through the
bundled nameserver (`docs/self-hosted-dns.md`), which needs no key of a DNS
provider. With DNS elsewhere, give dehydrated a DNS-01 hook for your provider
there; the codenames' address records are then yours to publish as well.
HTTP-01 is not set up: Caddy holds port 80 and serves no challenge for the
pool's hosts. The lab path (`KC_INSECURE=1`) self-signs instead of touching a
CA.

Let's Encrypt caps 50 certs/registered-domain/week, so one batch is at most 50
(10 is plenty for a pilot). `codenames.txt` holds the names to draw from.

## Isolation

Each real tenant's `castle` + `postgres` sit on their **own internal network**:
only that tenant's oauth2-proxy joins it, no other network is routed to it, and
the two reach nothing outside it. Castle also reads who is asking only from a
request that carries the secret it shares with that proxy, which `castlectl`
generates per codename. Keycloak's database is on an internal network of its
own, with Keycloak alone. Nothing crosses between tenants. This is
shared-kernel container isolation: add gVisor/Kata or a VM per tenant if a
client's threat model demands a kernel boundary.

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
castlectl                 certificates, pool, promote/demote, upgrade, backup/restore, DNS, status
summary.jq                what a tenant's castle may say about itself, for `castlectl status`
status.jq                 the document `castlectl status` prints, made from what it gathers
keycloak-realm.sh         a codename's Keycloak realm, its people, castlectl's own account
codenames.txt             the names every tenant and decoy is drawn from
branding/                 a file named `logo` here replaces the built-in brand (git-ignored)
dehydrated/               ACME client config and the DNS-01 hook
dns/                      the bundled nameserver's templates
systemd/                  the backup, the renewal and the status timer
docs/                     host setup, DNS, upgrades and backups, moving and retiring a tenant, Keycloak's administrators, the status
.env.example              configuration
.env                      yours, from the example (git-ignored)
.instances/               what castlectl generates: per-codename secrets and rendered files (git-ignored)
backups/                  encrypted bundles (git-ignored)
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

With the host prepared (`docs/host-setup.md`), the configuration comes first,
because every `castlectl` command reads it:

```bash
cp .env.example .env && $EDITOR .env                 # domain, Keycloak admin, etc.
cp dehydrated/config.example dehydrated/config && $EDITOR dehydrated/config
```

Then the order depends on who answers DNS for the pool.

**With the bundled nameserver** it has to be running before a certificate can
be issued, so the platform starts first. `docs/self-hosted-dns.md` has the DNS
steps (`dns-init`, its values into `.env`, the delegation); then:

```bash
podman compose -f platform.compose.yml --profile self-dns up -d
./castlectl issue-certs 10                            # one bulk cert batch for the pool
./castlectl provision-pool                            # stand up the 10 canaries
./castlectl list
```

**With DNS elsewhere** (a DNS-01 hook of your own in `dehydrated/config`, and
address records for the codenames) the certificates need nothing of the
platform and can come first:

```bash
./castlectl issue-certs 10
podman compose -f platform.compose.yml up -d
./castlectl provision-pool
./castlectl list
```

`provision-pool` creates a realm per codename, so it needs Keycloak, which
takes a minute or so to answer on its first start. The command waits for it,
two minutes at most. (With `ENGINE=docker` it is `docker compose` in place of
`podman compose`. Podman is what this deployment is validated on.)

Before a client's data arrives: give `castlectl` its own Keycloak account and
replace the first-boot admin (`docs/admin-rotation.md`, which also says how to
reach the admin console: the sign-in host serves it to the server itself
only), set up backups and try a restore (`docs/backup-and-upgrade.md`), enable
the renewal timer (above), and enable start on boot (`docs/host-setup.md`).

## Day to day

| To | Run |
|----|-----|
| see every codename and whether it is a tenant, a canary or only has a cert | `./castlectl list` |
| take a client on | `./castlectl promote <codename>`, then add their people (below) |
| see how the server and every instance on it are, as JSON for the overview | `./castlectl status` (`docs/status.md`, [`../overview/`](../overview/README.md)) |
| see who is in a codename's realm, and with which role (`none` for someone in no castle group, whom castle refuses) | `./castlectl users <codename>` |
| give `castlectl` an account of its own at Keycloak | `./castlectl keycloak-setup` (`docs/admin-rotation.md`) |
| move a tenant to another codename or host | `./castlectl export <codename>`, `./castlectl import <bundle> <codename>` (`docs/moving-and-retiring-a-tenant.md`) |
| end a tenant: export it, then destroy its data, people and secrets and leave a canary | `./castlectl demote <codename> --yes` |
| remove a canary and its realm, keeping the cert for reuse | `./castlectl deprovision <codename>` |
| add one more codename to the pool | `./castlectl allocate` |
| roll out a new castle image, proxy version or vhost template | `./castlectl upgrade` (`docs/backup-and-upgrade.md`) |
| back up, restore | `./castlectl backup`, `./castlectl restore <file> --yes` (`docs/backup-and-upgrade.md`) |
| renew the pool's certificates, all together | `./castlectl renew-pool`, which the renewal timer runs every week (*The certificate strategy*, above) |
| put the DNS zone back in line with what runs | `./castlectl dns-sync` (`docs/self-hosted-dns.md`) |

**People and roles.** Accounts live in the codename's Keycloak realm, and the
group an account is in gives its role. A manager then puts people on projects
in the portal. Adding and removing people, sessions and what ends them are in
[`../oauth2-proxy-keycloak.md`](../oauth2-proxy-keycloak.md).

**Your own logo.** Put an image file named `logo` in `branding/` (PNG, JPEG,
GIF, WebP or SVG, at most 1 MiB) and run `./castlectl upgrade`. Castle does not
run as root, so the file has to be readable by everyone (`chmod 644
branding/logo`); `castlectl` says so before it touches a tenant. The file is
git-ignored and is not part of a backup bundle, so keep a copy. To go back to
the built-in brand, delete the file and run `./castlectl upgrade` again: a
tenant that was started with a logo does not start without it.

**What a decoy records.** Nobody has an account at a canary, so anything that
arrives there is worth a look. Caddy logs every request as JSON on its standard
output (`podman logs compose_caddy_1`), and Keycloak keeps the sign-in attempts
at each codename's realm as events for seven days (admin console, the realm,
*Events*).
