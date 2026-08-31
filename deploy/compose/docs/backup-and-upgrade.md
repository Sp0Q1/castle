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

- **Postgres *major* upgrades are not a tag bump.** A new major won't boot on the
  old major's data dir ("database files are incompatible"), so it needs a
  dump/restore (or `pg_upgrade`) for keycloak-db and every tenant DB. Minor (18.x)
  bumps are free. The pins are on **18**; treat the *next* major as a deliberate
  migration — see **Migrating a Postgres major** below. Dependabot surfaces a pg
  major as a PR: that's your cue to plan the migration, not to blind-merge it.
- **Keycloak major upgrades** (e.g. 26 → 27): bump the image, but read the release
  notes first — Keycloak migrates its own schema, but majors can change config. If
  you built the optimized image (deploy/k8s/keycloak/Dockerfile), rebuild it on the
  new base.

Always `./castlectl backup` before a schema-changing release, and test it on a
staging box.

## Migrating a Postgres major

A pg major rewrites the on-disk format, so the move is: dump the logical data on
the old major, recreate the DB container on the new major with a **fresh** volume,
and load the dump back. Do it in a maintenance window. keycloak-db is always
present; tenant DBs exist only for promoted codenames (canaries have no DB):

```bash
cd /opt/castle/deploy/compose

# 1. Dump each live DB WHILE STILL ON THE OLD MAJOR (logical dump = portable).
podman exec compose_keycloak-db_1 pg_dump --clean --if-exists -U keycloak -d keycloak \
  | gzip > /root/kc-pre-migration.sql.gz
for cn in $(./castlectl list | awk '$2=="tenant"{print $1}'); do
  podman exec "postgres-$cn" pg_dump --clean --if-exists -U castle -d castle \
    | gzip > "/root/tenant-$cn.sql.gz"
done

# 2. Pull the new pins (postgres:NN-alpine bumped across compose + k8s).
git pull

# 3. Recreate keycloak-db on the new major with an empty volume, then load the dump.
podman compose -f platform.compose.yml stop keycloak keycloak-db
podman volume rm compose_keycloak_db          # the OLD-major data dir (confirm: podman volume ls)
podman compose -f platform.compose.yml up -d keycloak-db
until podman exec compose_keycloak-db_1 pg_isready -U keycloak >/dev/null 2>&1; do sleep 2; done
gunzip -c /root/kc-pre-migration.sql.gz | podman exec -i compose_keycloak-db_1 psql -U keycloak -d keycloak

# 4. Per promoted tenant (skip if all-canary): stop its castle, wipe its pg volume,
#    up -d postgres-<cn>, load /root/tenant-<cn>.sql.gz, then ./castlectl upgrade <cn>.

# 5. Bring the platform back and verify.
podman compose -f platform.compose.yml up -d keycloak
podman logs -f compose_keycloak_1             # wait until healthy
./castlectl list
```

Keep the `/root/*-pre-migration.sql.gz` dumps until the new major is proven
healthy — they're the rollback (recreate on the old tag, load the dump). Delete
them once verified.

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

## Scheduling (unattended)

`castlectl backup` is meant to run from a timer. Units live in
`deploy/compose/systemd/`:

```bash
# paths in the unit assume /opt/castle — edit them if you cloned elsewhere
cp deploy/compose/systemd/castle-backup.{service,timer} /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now castle-backup.timer
systemctl list-timers castle-backup.timer   # confirm the next run
journalctl -u castle-backup.service          # inspect the last run
```

It fires daily at 03:30 (randomized ±30m; `Persistent=true` catches a run the box
slept through). Run one by hand any time with `systemctl start
castle-backup.service` — or just `./castlectl backup`.

**Retention.** `BACKUP_KEEP` (default 14) prunes the local bundle dir to the
newest N after each run so it can't fill the disk; `0` keeps everything.

**Off-box copy.** Set `BACKUP_PUSH_CMD` to ship each new bundle somewhere durable.
It runs once per bundle with `{}` replaced by the bundle path (or the path
appended when there's no `{}`):

```bash
BACKUP_PUSH_CMD='rclone copy {} b2:castle-backups/'       # any rclone remote
BACKUP_PUSH_CMD='rsync -a {} backups@vault:/srv/castle/'  # another host over ssh
```

The bundles are already age-encrypted to your offline key, so the destination
never sees plaintext — object storage, another VPS, or a NAS are all fine. If the
push fails, the bundle is kept locally and the run exits non-zero (so the timer's
`journalctl` flags it) rather than being pruned away.

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
