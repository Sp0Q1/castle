# Host setup (do this once on a fresh VPS)

The bring-up in the README assumes the host is prepared. On a fresh Debian 13
box these are the prerequisites — a couple of them are non-obvious and cost real
debugging time, so the *why* is spelled out where it matters.

Run everything below as root.

## 1. Packages

```bash
apt update && apt -y upgrade
apt -y install git age podman podman-compose bind9-dnsutils curl ca-certificates runc
```

`podman` + `podman-compose` are the container engine (this deployment is
validated on podman 5.x / podman-compose 1.3.x — the versions Debian 13 ships).
`age` is for `castlectl backup`, `bind9-dnsutils` gives `dig` + `nsupdate` (the
self-hosted-DNS path uses them), and `runc` is the container runtime — see next.

## 2. Container runtime: runc, not crun (REQUIRED)

podman on Debian defaults to **crun**. crun stacks each container's processes
into an AppArmor child profile (`containers-default//&crun`) that the default
profile won't let them signal — which **breaks PostgreSQL**: its processes wake
each other with `SIGURG`, those signals get denied, and the database crash-loops
with `could not signal for checkpoint: Permission denied`, never becoming healthy.

**runc** doesn't do that profile stacking, so AppArmor stays fully enforced *and*
Postgres works. Make it the default runtime:

```bash
mkdir -p /etc/containers/containers.conf.d
printf '[engine]\nruntime = "runc"\n' > /etc/containers/containers.conf.d/runtime.conf
```

(No daemon to restart — podman reads this when it creates containers. If you set
it after already starting containers, re-create them: `podman compose … down` then
`up`.)

The alternative — `security_opt: apparmor=unconfined` on just the Postgres
services — also works but drops AppArmor for those containers. runc keeps every
container confined, so it's the one we use.

## 3. Free port 53 (only if you self-host DNS — Option 3)

If you're running castle's own authoritative nameserver (`SELF_HOSTED_DNS=1`,
the `--profile self-dns` service), the box's own `systemd-resolved` sits on port
53 and will collide with it. Turn off just its stub listener — resolved still does
the host's *own* name resolution, it just stops squatting on `:53`:

```bash
printf '[Resolve]\nDNSStubListener=no\n' > /etc/systemd/resolved.conf.d/no-stub.conf
ln -sf /run/systemd/resolve/resolv.conf /etc/resolv.conf     # host resolves via the real upstreams
systemctl restart systemd-resolved
# verify: :53 free, and the host still resolves names
ss -ulnp 'sport = :53' | grep -q ':53' && echo "still occupied" || echo ":53 free"
getent hosts github.com >/dev/null && echo "host DNS ok"
```

Skip this entirely if you manage DNS elsewhere (no self-hosted nameserver).

## 4. Firewall

Open the ports the platform serves on — and, for self-hosted DNS, 53 on **both**
UDP and TCP (TCP matters: large answers and Let's Encrypt's checks fall back to
it). Keep 22 for SSH.

| Port | Proto | For |
|------|-------|-----|
| 22   | TCP   | SSH |
| 80   | TCP   | ACME HTTP-01 (the IdP cert) + Caddy |
| 443  | TCP   | Caddy (all tenant/canary/IdP HTTPS) |
| 53   | UDP+TCP | self-hosted authoritative DNS (Option 3 only) |

On TransIP this is the control-panel firewall; on the box itself a fresh Debian
has no local firewall by default.

## 5. Clone

```bash
git clone https://github.com/Sp0Q1/castle.git /opt/castle
cd /opt/castle/deploy/compose
```

The deploy configs reference `/opt/castle`, so clone there.

## Verify, then continue

```bash
podman --version                 # 5.x
podman compose version 2>&1 | tail -1   # podman-compose 1.3.x
command -v runc                  # present
```

From here:
- **Config:** `cp .env.example .env` and fill it (see `.env.example`).
- **Self-hosted DNS:** `docs/self-hosted-dns.md` (delegation, `dns-init`).
- **Certificates + bring-up:** the README's *Usage* section.
- **Backups + upgrades:** `docs/backup-and-upgrade.md`.
