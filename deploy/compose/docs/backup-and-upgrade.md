# Upgrading, backing up, and restoring

One tenant on its own, to move or retire it: [`moving-and-retiring-a-tenant.md`](moving-and-retiring-a-tenant.md).

## Upgrading

An upgrade has two parts: the platform (Caddy, Keycloak and its database,
BIND) and the instances (every tenant and canary).

```bash
cd /opt/castle/deploy/compose
git pull                          # templates, version pins, castlectl itself
./castlectl backup                # before anything changes

# the platform
podman compose -f platform.compose.yml --profile self-dns pull
podman compose -f platform.compose.yml --profile self-dns up -d --force-recreate

# the instances
./castlectl upgrade               # every running one (or: upgrade <codename>)
./castlectl list
```

Then sign in once. (Leave out `--profile self-dns` if you do not run the
bundled nameserver.)

**The platform needs `--force-recreate`.** podman-compose replaces a container
only when its definition changed. A change to `caddy/Caddyfile` needs it too:
the file is mounted on its own, so the running Caddy keeps the one it started
with. A newer image pulled under the same tag is
not such a change, so without the flag the old containers keep running. While
it recreates, it may report that a network "has associated containers": the
instances' proxies are attached to it, and they stay up.

**`castlectl upgrade`** renders every running instance again from the
templates, rewrites its vhost, pulls, and recreates what changed. It then
brings the attributes of the client in the instance's realm (the PKCE
requirement among them) to what `keycloak-realm.sh create` sets today, so
realms made at different times do not differ in them. For that it needs
Keycloak, which the platform step has just recreated, so it first waits until
Keycloak answers, two minutes at most. **Data volumes are kept**, and castle
applies pending schema migrations when it starts (`auto_migrate: true`). One
instance that fails does not stop the others; the summary at the end names it
and the exit status is non-zero.

`CASTLE_IMAGE` in `.env` names the castle image. `ghcr.io/sp0q1/castle:main`
follows the `main` branch. `upgrade` resolves it to a digest once, so every
tenant gets the same build, and prints the first twelve characters of the
digest it moved each tenant to. Compare that with the build you meant to
deploy. Setting `CASTLE_IMAGE` to a digest pins that build, for a rollback.

The templates and the image are made for each other. When `git pull` brings
new templates, deploy the image built from the same commit, not an older pin:
an image and a template that disagree on how the proxy and castle talk leave
the tenant refusing every request until they match again.

Two things an upgrade does **not** paper over:

- **Postgres *major* upgrades are not a tag bump.** A new major won't boot on the
  old major's data dir ("database files are incompatible"), so it needs a
  dump/restore (or `pg_upgrade`) for keycloak-db and every tenant DB. Minor (18.x)
  bumps are free. The pins are on **18**; treat the *next* major as a deliberate
  migration — see **Migrating a Postgres major** below. Dependabot surfaces a pg
  major as a PR: that's your cue to plan the migration, not to blind-merge it.
- **Keycloak major upgrades** (e.g. 26 → 27): bump the image, but read the release
  notes first — Keycloak migrates its own schema, but majors can change config.

Always `./castlectl backup` before a schema-changing release, and test it on a
staging box.

## Migrating a Postgres major

A pg major rewrites the on-disk format, so the move is: dump the logical data on
the old major, start the database on the new major with an empty data
directory, and load the dump back. Do it in a maintenance window: people are
signed out and the portals are down for some minutes. keycloak-db is always
present; tenant DBs exist only for promoted codenames (canaries have no DB).

The old major's files are not deleted. They are moved aside inside their
volume, and stay there until the new major is proven.

```bash
cd /opt/castle/deploy/compose
./castlectl backup                # as before any upgrade

# 1. Dump every database while it still runs on the old major.
podman exec compose_keycloak-db_1 pg_dump --clean --if-exists -U keycloak -d keycloak \
  | gzip > /root/kc-pre-migration.sql.gz
tenants="$(./castlectl list | awk '$2=="tenant"{print $1}')"
for cn in $tenants; do
  podman exec "postgres-$cn" pg_dump --clean --if-exists -U castle -d castle \
    | gzip > "/root/tenant-$cn-pre-migration.sql.gz"
done
ls -l /root/*-pre-migration.sql.gz        # one file per database, none of them empty

# 2. The new pins.
git pull

# 3. Keycloak's database. Its two containers go, the old major's files are
#    moved aside inside the volume, and the database starts alone on the new
#    major to take the dump.
podman stop compose_keycloak_1 compose_keycloak-db_1
podman rm compose_keycloak_1 compose_keycloak-db_1
kc_vol="$(podman volume inspect -f '{{.Mountpoint}}' compose_keycloak_db)"
mv "$kc_vol/pgdata" "$kc_vol/pgdata.old-major"
podman compose -f platform.compose.yml up -d keycloak-db
until podman exec compose_keycloak-db_1 pg_isready -h 127.0.0.1 -U keycloak -d keycloak >/dev/null 2>&1; do sleep 2; done
gunzip -c /root/kc-pre-migration.sql.gz \
  | podman exec -i compose_keycloak-db_1 psql -v ON_ERROR_STOP=1 -U keycloak -d keycloak >/dev/null

# 4. The platform, as in any upgrade.
podman compose -f platform.compose.yml --profile self-dns pull
podman compose -f platform.compose.yml --profile self-dns up -d --force-recreate

# 5. Each tenant's database, the same way. `upgrade` recreates the tenant on
#    the new pin with an empty database, and the dump is loaded while castle
#    is stopped.
for cn in $tenants; do
  podman stop "castle-$cn" "postgres-$cn"
  vol="$(podman volume inspect -f '{{.Mountpoint}}' "castle-${cn}_castle_db")"
  mv "$vol/pgdata" "$vol/pgdata.old-major"
  ./castlectl upgrade "$cn"
  podman stop "castle-$cn"
  gunzip -c "/root/tenant-$cn-pre-migration.sql.gz" \
    | podman exec -i "postgres-$cn" psql -v ON_ERROR_STOP=1 -U castle -d castle >/dev/null
  podman start "castle-$cn"
done

# 6. The canaries, and the summary.
./castlectl upgrade
./castlectl list
```

Then sign in at every tenant and look at a project. (Leave out
`--profile self-dns` if you do not run the bundled nameserver.)

Two things in these steps are there for a reason. Keycloak's container is
removed before its database's, because podman does not remove a container
that another one depends on. And a tenant's containers are left for
`castlectl upgrade` to replace: it only upgrades what it finds running.

Until the new major is proven, each volume holds both: the new `pgdata` and
the old major's `pgdata.old-major`, and the dumps are in `/root`. Going back is
the previous commit's pins with `pgdata.old-major` under its old name again.
Once it is proven:

```bash
rm -rf "$kc_vol/pgdata.old-major"
for cn in $tenants; do
  rm -rf "$(podman volume inspect -f '{{.Mountpoint}}' "castle-${cn}_castle_db")/pgdata.old-major"
done
rm /root/*-pre-migration.sql.gz
```

## Backups

`castlectl backup` writes one **age-encrypted** bundle containing everything that
can't be re-derived:

- every tenant's DB (logical `pg_dump`, so it's portable across pg versions) and
  its uploads,
- Keycloak's realms/users,
- the per-codename secrets (`.instances/`), the whole pre-issued cert pool, the
  dehydrated account, and `.env`.

**It bundles each tenant's DB together with its secrets and cert**, so a restore
never leaves a database without the secrets that make it usable.

Not in the bundle: a logo in `branding/`. Keep a copy of that file.

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

Run it from the timer below, and copy the `.age` files off-box. This is a
*complement* to VPS snapshots: snapshots are your whole-box disaster recovery;
these bundles add portability and off-provider safety. They carry the pool's
certificates, so a restore asks no CA for the pool and adds no Certificate
Transparency entries for it. The sign-in host's certificate is Caddy's own and
is not in the bundle: on a new server Caddy obtains it again. `restore` brings
back the whole deployment; one client on its own is `castlectl export` and
`import` ([`moving-and-retiring-a-tenant.md`](moving-and-retiring-a-tenant.md)).

## Scheduling (unattended)

`castlectl backup` is meant to run from a timer. Units live in
`deploy/compose/systemd/`:

```bash
# paths in the unit assume /opt/castle — edit them if you cloned elsewhere
cp systemd/castle-backup.{service,timer} /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now castle-backup.timer
systemctl list-timers castle-backup.timer   # confirm the next run
journalctl -u castle-backup.service          # inspect the last run
```

It fires daily at 03:30, delayed by up to 30 minutes (`Persistent=true` catches
a run the box slept through). Run one by hand any time with `systemctl start
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
podman compose -f platform.compose.yml --profile self-dns up -d
./castlectl restore backups/castle-backup-<ts>.tar.age --yes   # now loads Keycloak realms/users
./castlectl provision-pool                                     # every codename comes up as a canary
./castlectl promote <codename>                                 # once per tenant, on its restored secrets
./castlectl restore backups/castle-backup-<ts>.tar.age --yes   # loads each tenant DB + uploads
```

On a server with another address than the one the bundle came from, change
`PUBLIC_IP` in the restored `.env` before the platform starts (and the address
in `KC_ADMIN_FROM`, if one is set there), and point the sign-in host's record
and the delegation's glue record at the new address. With the bundled
nameserver each codename's record is published as it is provisioned.

Re-running is harmless: each pass loads whatever is now reachable and names
the tenants it could not reach yet. The `--yes` flag is required because a
restore overwrites secrets/certs and reloads databases.

Try this on a spare machine before you depend on it, and again after a change
to the backup or the templates.
