# Moving and retiring a tenant

`castlectl backup` is the whole host in one bundle. These commands work on one
tenant.

| To | Run |
|----|-----|
| see who is in a codename's realm and with which role | `./castlectl users <codename>` |
| write one tenant's data and people as an encrypted bundle | `./castlectl export <codename>` |
| bring that tenant up under a codename that is not in use | `./castlectl import <bundle> <codename>` |
| end a tenant | `./castlectl demote <codename> --yes` |

## What a tenant bundle holds

The tenant's database, its uploads, and its people: who is in the realm and in
which group. It holds no password, no authenticator secret and none of the
codename's own secrets. It is encrypted to `BACKUP_AGE_RECIPIENT` like a backup,
lands in `BACKUP_DIR` as `castle-tenant-<codename>-<time>.tar.age`, and goes
through `BACKUP_PUSH_CMD` if that is set. Tenant bundles are not pruned.

## Retiring a tenant

```bash
./castlectl demote <codename> --yes
```

It stops the tenant's castle, writes a tenant bundle, and then removes the
realm with everyone in it, the database and uploads, and the codename's
secrets. What is left is a canary like any other: an empty realm, new secrets,
the same certificate and name. Nobody's session survives it.

If the export or Keycloak fails, nothing is destroyed and the tenant keeps
running. `--no-export` skips the bundle, for a tenant whose data nobody needs.
`deprovision` refuses a tenant, so this is the one command that destroys a
client's data.

## Moving a tenant to another codename

```bash
./castlectl export <old>                       # the tenant keeps running
export BACKUP_AGE_IDENTITY=/path/to/castle-backup.key
./castlectl import backups/castle-tenant-<old>-<time>.tar.age <new>
# look at <new>, give its people passwords, tell them the new address
./castlectl demote <old> --yes                 # writes a last bundle, then retires <old>
```

`<new>` has to be a canary. `import` promotes it, loads the database and the
uploads, and creates the people in its realm, in the groups they were in. They
arrive **without credentials**: give each one a temporary password in Keycloak
before they sign in. Their projects, findings and comments are theirs again at
that first sign-in. Castle binds an account to the id Keycloak has for a
person, and in the new realm everybody has a new one, so `import` lets every
account go (`user:unbind all:yes`); each is then taken by the person the new
realm vouches for with its address.

Anything written at `<old>` after the export is not in the bundle. To lose
nothing, tell people to stop first, or import the bundle that `demote` writes
instead of an earlier one.

The same steps move a tenant to another host: copy the bundle there and import
it. The castle on the receiving side has to be the same version or newer.

`import` needs the age identity, as `restore` does. Bring it to the host for the
import and take it away again.
