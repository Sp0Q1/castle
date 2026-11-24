# Keycloak's administrators (do this once, before real client data)

## Reaching the admin console

To the world the sign-in host serves signing in and nothing else of Keycloak.
The master realm, the admin console and the admin API answer only a request
that starts on the server itself (`caddy/sites/sso.caddy`); from anywhere else
they are 404. `castlectl` asks that way by itself. A browser's requests start
on the server when they go through an SSH tunnel.

Through the tunnel the server asks itself by its public name, so its own
public address has to count as the server's. Once, in `.env`:

```bash
KC_ADMIN_FROM="private_ranges <the server's public address>"
```

then `podman compose -f platform.compose.yml up -d caddy`, which recreates
Caddy and nothing else; the portals and the sign-in host are away for the
seconds that takes.

The tunnel, from your own machine:

```bash
ssh -D 1080 -N root@<server>
```

It says nothing and stays in the foreground for as long as the tunnel is open;
Ctrl-C closes it. In a second terminal, start a browser window that sends
everything through the tunnel. Chrome and Chromium do that with two options,
in a profile of its own that leaves the browser you use otherwise as it is:

```bash
# Linux (google-chrome takes the same options)
chromium --user-data-dir="$HOME/.castle-admin" --proxy-server="socks5://localhost:1080" https://<KC_HOST>/admin/
# macOS
open -na "Google Chrome" --args --user-data-dir="$HOME/.castle-admin" --proxy-server="socks5://localhost:1080" https://<KC_HOST>/admin/
```

What that window keeps (cookies, history) is in `~/.castle-admin` and nowhere
else: delete the directory when you are done. Any other browser works with a
profile whose proxy is `SOCKS5 localhost:1080`, with name lookups through the
proxy. Until the rotation below is done, the account to sign in with is the
first-boot admin: `KC_ADMIN_USER` and `KC_ADMIN_PASS` in `.env`.

## castlectl's own account

`castlectl` creates and deletes realms, so it signs in to Keycloak. Out of the
box it does that as the first-boot admin, with the password in `.env`. Give it
an account of its own before anything else on this page:

```bash
./castlectl keycloak-setup
```

That makes a client `castlectl` in the master realm with a secret, no browser
sign-in and the admin role, keeps the secret in `.instances/`, and switches on
the lock-out after failed sign-ins for the master realm. From then on
`castlectl` needs no administrator's password, and the rest of this page does
not affect it.

## Rotating the first admin

A fresh deployment boots Keycloak with a **first-boot admin** created from
`KC_BOOTSTRAP_ADMIN_USERNAME`/`_PASSWORD` (set in `.env` as `KC_ADMIN_USER` /
`KC_ADMIN_PASS`). That account is fine for setup but is the wrong thing to keep:
it's a **shared, static, single-factor** credential whose password sits in plain
`.env` on the box. Before any client's people are in Keycloak's realms,
replace it with a **named admin that has TOTP**, then **delete the bootstrap
account**.

Keycloak reads `KC_BOOTSTRAP_ADMIN_*` **on its first start only, while there is
no master realm yet**. After that it ignores them, and deleting the bootstrap
user does **not** bring it back. So this rotation is permanent, and the values
left in `.env` belong to no account.

### Steps

Do these in the admin console (see above for reaching it), signed in to the
**`master`** realm as the bootstrap admin.

1. **Create the named admin.** `master` realm → **Users** → **Add user**.
   Set a username and a real email, toggle **Email verified** on, **Create**.
2. **Set its password.** The new user → **Credentials** → **Set password**.
   Use a strong, unique password, turn **Temporary** **off**, **Save**, and
   confirm with **Save password**.
3. **Grant admin.** The user → **Role mapping** → **Assign role** → **Realm
   roles** → tick **`admin`** → **Assign**. (A `master`-realm user with the
   `admin` role administers every realm, including the `castle-*` tenant
   realms.)
4. **Force TOTP enrolment.** The user → **Details** → **Required user actions** →
   add **Configure OTP** → **Save**. This makes the next login enrol an
   authenticator before it grants access.
5. **Verify — this is the gate before deleting anything.** Open a private
   window of that same browser (it goes through the tunnel as well) and sign in
   to `master` as the named admin. Keycloak shows **Mobile Authenticator
   Setup**: scan the code with your authenticator app, enter the one-time code
   it gives and **Submit**. Confirm you land in the console and that **Manage
   realms** lists the `castle-*` realms. **Do not proceed until this works.**
6. **Delete the bootstrap admin.** In that private window, as the named admin:
   `master` → **Users** → the first-boot admin (the name in `KC_ADMIN_USER`) →
   **Action** → **Delete**. From here on there is exactly one admin, and it has
   a second factor.

### The password that is left in `.env`

After step 6 the two values in `.env` belong to no account, and no start of
Keycloak reads them again. The lines stay, because `platform.compose.yml` asks
for them, which is what stops a first start that forgot them. Put a long
random value that nobody keeps in the place of the password:

```bash
sed -i "s|^KC_ADMIN_PASS=.*|KC_ADMIN_PASS=$(openssl rand -hex 32)|" .env
```

Nothing has to be restarted for it. The former password is gone from `.env` at
once; Keycloak's container keeps it among its settings until the platform's
containers are next made anew, which an update of the platform does
([`backup-and-upgrade.md`](backup-and-upgrade.md)). Do this after
`castlectl keycloak-setup` (above): until then `castlectl` signs in with that
password.

### Prefer the CLI?

The account steps (1–4) can be scripted with `kcadm.sh` inside the running
container; TOTP enrolment (step 5) is interactive by design and still done in the
browser. There is no `castlectl` command for it: a one-time, security-critical
rotation is clearest done deliberately by hand, and `castlectl` itself no
longer depends on that account once it has its own.

### Recovery

Locked out, because the one admin's authenticator is lost? Keycloak makes a
temporary admin from its command line while the server itself is stopped.
Nobody can sign in while it is. The one-off runs the Keycloak that
`platform.compose.yml` names, so do this on a platform that is up to date
with its files ([`backup-and-upgrade.md`](backup-and-upgrade.md)):

```bash
cd /opt/castle/deploy/compose
podman compose -f platform.compose.yml stop keycloak
podman compose -f platform.compose.yml run --rm --no-deps -T keycloak bootstrap-admin user \
  --username recovery --password:env KC_BOOTSTRAP_ADMIN_PASSWORD --no-prompt </dev/null
podman compose -f platform.compose.yml start keycloak
podman compose -f platform.compose.yml ps
```

Among what the `run` line prints is `Created temporary admin user with
username recovery`. If it says instead that a user of that name exists, the
account is one left from an earlier time, with the password it had then: run
the line with another `--username`. The line leaves every other container as
it is (`--no-deps`) and reads nothing from the terminal, so the lines after it
run also when the block is typed in at once. The last line lists the
platform's containers, Keycloak among them and up again; if the session was
cut off before `start`, that is the line to run.

The account's password is the value of `KC_ADMIN_PASS` in `.env`, which the
container holds under the name in the command. (A variable of that name in
your shell would take its place.)

In the console, as `recovery`: **Users** → the named admin → **Credentials** →
the menu at the end of the **Otp** row → **Delete**; then **Details** →
**Required user actions** → **Configure OTP** → **Save**. (A forgotten
password is **Reset password** in the same table.) Sign in as the named admin
in a private window and set up the new authenticator, as in step 5. Then, as
the named admin, delete `recovery` as in step 6 and replace the password in
`.env` once more, with the `sed` line above.

Two named admins, each with an authenticator of their own, never need this:
make the second one as soon as there is a second operator.
