# Rotating the Keycloak admin (do this once, before real client data)

A fresh deployment boots Keycloak with a **first-boot admin** created from
`KC_BOOTSTRAP_ADMIN_USERNAME`/`_PASSWORD` (set in `.env` as `KC_ADMIN_USER` /
`KC_ADMIN_PASS`). That account is fine for setup but is the wrong thing to keep:
it's a **shared, static, single-factor** credential whose password sits in plain
`.env` on the box. Before any client's findings live in Keycloak's realms,
replace it with a **named admin that has TOTP**, then **delete the bootstrap
account**.

Keycloak only reads `KC_BOOTSTRAP_ADMIN_*` **on the first start of an empty
instance** — once any admin exists it is ignored, and deleting the bootstrap
user does **not** bring it back. So this rotation is permanent, and the leftover
`.env` values become inert (a dead credential mapping to no account).

## Steps

Do these in the admin console at `KC_PUBLIC_URL` (e.g. `https://sso.example.com`),
signed in to the **`master`** realm as the bootstrap admin.

1. **Create the named admin.** `master` realm → **Users** → **Add user**.
   Set a username and a real email, toggle **Email verified** on, **Create**.
2. **Set its password.** The new user → **Credentials** → **Set password**.
   Use a strong, unique password and turn **Temporary** **off**.
3. **Grant admin.** The user → **Role mapping** → **Assign role** → switch the
   filter to **Filter by realm roles** → tick **`admin`** → **Assign**. (A
   `master`-realm user with the `admin` role administers every realm, including
   the `castle-*` tenant realms.)
4. **Force TOTP enrolment.** The user → **Details** → **Required user actions** →
   add **Configure OTP** → **Save**. This makes the next login enrol an
   authenticator before it grants access.
5. **Verify — this is the gate before deleting anything.** Open a private window,
   sign in to `master` as the named admin. You'll be prompted to scan the TOTP QR
   with your authenticator; complete it. Confirm you land in the console and can
   see the `castle-*` realms. **Do not proceed until this works.**
6. **Delete the bootstrap admin.** Back as the named admin: `master` → **Users** →
   the original bootstrap `admin` → **Delete**. From here on there is exactly one
   admin, and it has a second factor.

## Clean up the dead credential (optional but tidy)

After step 6 the `.env` bootstrap values map to nothing and are never re-read, so
this is hygiene, not security-critical. To remove them:

```bash
# in .env — delete or blank these two lines:
#   KC_ADMIN_USER=...
#   KC_ADMIN_PASS=...
```

Because `platform.compose.yml` marks them required (`:?`) to catch a *first*-boot
that forgot them, also drop the two `KC_BOOTSTRAP_ADMIN_*` lines from the
`keycloak` service, then recreate it (Keycloak already has your named admin, so
nothing is lost):

```bash
podman compose -f platform.compose.yml up -d keycloak   # or: docker compose ...
```

## Prefer the CLI?

The account steps (1–4) can be scripted with `kcadm.sh` inside the running
container; TOTP enrolment (step 5) is interactive by design and still done in the
browser. Ask if you want a `castlectl rotate-admin` helper that does the kcadm
part — it's a small addition, left out here because a one-time, security-critical
rotation is clearest done deliberately by hand.

## Recovery

Locked out (lost the TOTP device with only one admin)? Keycloak can re-bootstrap:
stop the stack, run the keycloak image's one-shot
`bin/kc.sh bootstrap-admin user --username <tmp> --password:env <VAR>` against the
same DB volume to mint a temporary admin, sign in, fix the account, then delete
the temporary one. Keeping **two** named admins (each with its own TOTP) avoids
needing this at all — recommended once you have more than one operator.
