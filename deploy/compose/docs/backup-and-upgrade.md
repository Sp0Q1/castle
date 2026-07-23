# Upgrading, backing up, and restoring

## Upgrading

The castle app runs pending schema migrations on boot (`auto_migrate: true`), so
an app upgrade is just a new image plus a recreate:

```
# bump CASTLE_IMAGE in .env to the new tag/digest, then:
./castlectl upgrade            # every running instance (or: upgrade <codename>)
```

`upgrade` re-renders each running instance, pulls, and `up -d`s it — **data
volumes are kept**, and migrations apply on boot. It handles the per-tenant
fan-out for you.

Two things it does **not** paper over:

- **Postgres *major* upgrades are not a tag bump.** `postgres:16 → 17` changes
  the on-disk format, so it needs a dump/restore (or `pg_upgrade`) for keycloak-db
  and every tenant DB. Minor (16.x) bumps are free. Keep Postgres pinned to 16 and
  treat a major as a deliberate migration (a `backup` then restore into fresh 17
  volumes is the simplest path).
- **Keycloak major upgrades** (e.g. 26 → 27): bump the image, but read the release
  notes first — Keycloak migrates its own schema, but majors can change config. If
  you built the optimized image (deploy/k8s/keycloak/Dockerfile), rebuild it on the
  new base.

Always `./castlectl backup` before a schema-changing release, and test it on a
staging box.

## Backups

`castlectl backup` writes one **age-encrypted** bundle containing everything that
can't be re-derived:

- every tenant's DB (logical `pg_dump`, so it's portable across pg versions) and
  its uploads,
- Keycloak's realms/users,
- the per-codename secrets (`.instances/`), the whole pre-issued cert pool, the
  dehydrated account, and `.env`.

**It bundles each tenant's DB together with its secrets and cert**, so a restore
can never orphan the data — the trap where you restore a database but lose the
JWT/DB/oauth secrets that make it usable.

### Key custody (do this once, off the VPS)

```
age-keygen -o castle-backup.key        # on your workstation, NOT the VPS
grep 'public key' castle-backup.key    # -> age1...
```

Put the **public** key in `.env` as `BACKUP_AGE_RECIPIENT`. Keep
`castle-backup.key` (the private identity) **off the box**. The VPS can then write
backups but cannot decrypt them — a compromise of the VPS does not expose old
backups. `restore` needs the identity via `BACKUP_AGE_IDENTITY`.

```
./castlectl backup                     # -> ./backups/castle-backup-<ts>.tar.age
```

Schedule it from cron, and copy the `.age` files off-box (or let your VPS
snapshots carry them). This is a *complement* to VPS snapshots: snapshots are your
whole-box disaster recovery; these bundles add portability, per-tenant granular
restore, and off-provider safety — and, unlike re-issuing, they preserve the
existing certs so a restore creates **no new Certificate Transparency entries**.

## Restoring

`restore` is idempotent and safe to re-run; it restores the files, then loads the
DBs into whatever stacks are currently up (stopping each app for its own load so
it isn't mutating the DB mid-restore).

On a fresh box:

```
export BACKUP_AGE_IDENTITY=/path/to/castle-backup.key
./castlectl restore backups/castle-backup-<ts>.tar.age --yes   # restores secrets, certs, .env
docker compose -f platform.compose.yml --profile self-dns up -d
./castlectl restore backups/castle-backup-<ts>.tar.age --yes   # now loads Keycloak realms/users
./castlectl provision-pool                                     # brings tenant stacks up on the restored secrets
./castlectl restore backups/castle-backup-<ts>.tar.age --yes   # loads each tenant DB + uploads
```

Re-running is harmless: each pass loads whatever is now reachable and reports what
still needs a stack brought up. The `--yes` flag is required because a restore
overwrites secrets/certs and reloads databases.
