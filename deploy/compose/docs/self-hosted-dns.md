# Self-hosted authoritative DNS (Option 3)

Run castle's own authoritative nameserver for the tenant apex (`$BASE_DOMAIN`),
so cert issuance needs **no DNS-provider API key** and a dark codename leaves
**no standing DNS record**. This is the maximum-decoy-hygiene option: castle
controls the whole `$BASE_DOMAIN` zone, publishes A records only when a codename
is provisioned, and creates `_acme-challenge` TXT records only for the seconds an
ACME validation is in flight.

It's opt-in — the `dns` service only starts under the `self-dns` compose profile,
and castlectl touches DNS only when `SELF_HOSTED_DNS` is set. Leave it off to keep
managing DNS at your provider.

## Trade-off you're accepting

You become the authoritative DNS for everything under `$BASE_DOMAIN`. On a single
VPS that's the same failure domain as the app itself — but note: **if this box
(or the `dns` service) is down, tenants don't resolve *and* renewals can't run.**
The nameserver is authoritative-only (no recursion, response-rate-limited), so it
can't be abused as an open resolver or amplifier — but it must stay reachable on
`:53`. For real redundancy, add a secondary nameserver (see below).

## One-time setup

1. **Generate the shared TSIG secret** and set the knobs in `.env`:

   ```
   ./castlectl dns-init
   ```
   Copy its output into `.env`:
   ```
   SELF_HOSTED_DNS=1
   PUBLIC_IP=203.0.113.10        # this VPS's public IP
   DDNS_TSIG_SECRET=<from dns-init>
   ```
   The same secret is read by the `dns` service and by castlectl — one key, two
   consumers. `.instances/ddns.key` (the nsupdate key file) is derived from it and
   is git-ignored.

2. **Delegate `$BASE_DOMAIN` to this box** at your registrar / the parent zone
   (e.g. in the `example.com` zone if `BASE_DOMAIN=tenants.example.com`):

   ```
   tenants           IN NS  ns1.tenants.example.com.
   ns1.tenants       IN A   203.0.113.10          ; glue — the VPS public IP
   ```
   That glue A matches the `ns1` record the zone serves itself. Most registrars
   want **two** NS records; until you run a real secondary, you can point both
   `ns1` and `ns2` at the same IP, but that is not true redundancy.

3. **Open `:53`** (UDP *and* TCP) to the world on the VPS firewall. TCP matters —
   large answers and Let's Encrypt's checks fall back to it.

4. **Start the platform with the DNS profile:**

   ```
   docker compose -f platform.compose.yml --profile self-dns up -d
   ```

5. **Verify** from off-box:

   ```
   dig +short SOA tenants.example.com @203.0.113.10
   dig +short NS  tenants.example.com @203.0.113.10
   dig +short A   ns1.tenants.example.com @203.0.113.10
   ```

Then the normal flow works with no provider API key:

```
./castlectl issue-certs 10        # DNS-01 via nsupdate to your own server
./castlectl provision-pool        # each codename gets its A record automatically
```

## How it behaves day to day

- **Provisioning** a canary/tenant publishes `<codename>.$BASE_DOMAIN A $PUBLIC_IP`;
  **deprovisioning** withdraws it. Promotion/demotion keep the same name, so DNS is
  unchanged and the swap stays invisible.
- **`./castlectl dns-sync`** reconciles the zone to reality: an A record for every
  running codename, nothing for the rest (the runtime is the source of truth, same
  as `list`). Run it after a restore or if you ever suspect drift.
- **ACME** (`issue-certs` / `renew-pool`) drives `dehydrated`'s DNS-01 through
  `dehydrated/dns-hook.sh`, which adds the TXT, waits until the local server serves
  it, lets Let's Encrypt validate, then deletes it.
- Updates are authenticated with the TSIG key; an update signed with the wrong key
  is refused, so only castlectl and the ACME hook can write records.

## The IdP host is a separate apex

`KC_HOST` (e.g. `sso.example.com`) is on a different apex than `$BASE_DOMAIN`, so
this nameserver isn't authoritative for it and `issue-certs` skips it. The IdP is
always-on and not a decoy, so give it a cert one of two ways:

- **Caddy HTTP-01** for just the `sso` vhost (simplest — it's always listening), or
- add the IdP apex as a **second self-hosted zone** (another NS delegation + a
  `zone` block) if you'd rather keep everything on DNS-01.

## Caveats worth remembering

- **Let's Encrypt multi-perspective validation:** LE queries your authoritative
  server from several network vantage points. A normal public IP is fine; a
  firewall that only allows some regions is not.
- **Renewal depends on `:53` being up.** Batched renewal (`renew-pool`) needs the
  `dns` service reachable. Monitor it like any cert-critical service.
- **No AXFR by default** (`allow-transfer none`) — the zone (your whole codename
  pool) is never dumped to anyone. If you add a secondary, allow transfer *only*
  to it (ideally TSIG-signed), so you don't hand the pool to the world.
- **Secondary nameserver (recommended before real load):** run the VPS as a hidden
  primary and add a secondary (a second small box, or a free secondary-DNS service
  that AXFRs from you). That removes the single-NS SPOF while keeping you in control
  of the zone contents.
