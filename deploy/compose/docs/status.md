# Status for the overview

`./castlectl status` prints one JSON document that says how this server is.
The overview ([`../../overview/`](../../overview/README.md)) reads it from every
server and shows them on one page. Nothing here lets anyone change a server
from outside: the document is written on the server, and one account can read
it and do nothing else.

## What is in it

| Field | Says |
|-------|------|
| `generated_at` | when the document was written |
| `platform` | Caddy, Keycloak, its database and, with the bundled nameserver, `dns`: each `healthy`, `starting` or `unhealthy` |
| `sign_in_certificate_ends` | until when the certificate is valid that the sign-in host hands out |
| `instances` | every codename that runs or has a certificate: `tenant`, `canary` or `cert-only`, its health, and until when the certificate is valid that is handed out for it. For a tenant also the twelve characters `upgrade` prints for its image, and its summary |
| `newest_backup` | when the newest bundle in `BACKUP_DIR` was written |
| `host` | whether a restart is waiting, how many systemd units have failed, how full the disk with the volumes is, and whether the renewal and backup timers are active |

A tenant's summary is what its castle prints for `castle-cli task summary`:
for each project its id, how many findings are drafts and how many are
published by severity, the comments of the last seven days by what their
author is (client, staff or manager) and when something last happened; and the
newest fifty of those comments, each with its time, project, finding and that
role.

Every value is a number, `true` or `false`, a time, a codename, twelve
characters of a digest or one of a fixed list of words. No name, title or text
is in the document, and none is read to make it. A value that could not be
read is `null`, never a value that reads as fine.

What the document does say is which codenames are tenants. Treat the file, and
a key that can read it, like the list of your clients.

A tenant's castle is not taken at its word. At most a megabyte of what it
prints is read, and the summary is rebuilt field by field (`summary.jq`). If
anything in it is not what a summary has, that tenant's `summary` is `null`
and the rest of the document is as usual. A summary carries at most 500
projects; an instance with more says so instead of printing one.

The document itself is made in one place (`status.jq`) from what `castlectl`
gathers, and stays under a megabyte, which is what the overview reads of an
answer. Where it would not, the largest summaries are `null` first. A pool
name that is not of a codename's shape (a lower-case letter, then up to 30
lower-case letters, digits or hyphens) is not asked about and is not in the
document. Whenever something is left out, `castlectl` says how much on its
standard error, which the timer's unit writes to the journal
(`journalctl -u castle-status`).

A tenant's summary and the certificate served for a name are answers from
outside `castlectl`, and each can keep it waiting. Together they have two
minutes of a run (`STATUS_SECONDS`, 120). Past that the remaining ones are not
asked for and are `null`, and the run says how many, so that a server on which
much is slow still gives a status in time.

If the engine does not say what runs, or the pool's names cannot be read,
there is no document: the run fails, the file stays as it was, and its age
shows.

```bash
./castlectl status | jq .
```

## Writing it every five minutes

Once, as root, in `deploy/compose`:

```bash
getent group castle-status >/dev/null || groupadd --system castle-status
install -d -m 2750 -o root -g castle-status /var/lib/castle-status
cp systemd/castle-status.{service,timer} /etc/systemd/system/
systemctl daemon-reload
systemctl enable --now castle-status.timer
systemctl start castle-status.service
ls -l /var/lib/castle-status/
```

The last line shows `status.json`: root's, readable by the group
`castle-status` and by nobody else. The directory's mode gives every file in
it that group. Each run writes a new file and puts it in the old one's place in
one step. A run that fails, or has not ended after four minutes, leaves the
file as it was; its `generated_at` then grows old, and the overview says so
after fifteen minutes.

## The account that reads it

The block below needs OpenSSH 9.2 or newer (Debian 12 and later). It can be
run again: it makes what is not there yet and writes the rest anew.

```bash
getent group castle-status >/dev/null || groupadd --system castle-status
getent passwd castle-status >/dev/null || useradd --system --gid castle-status --home-dir /var/lib/castle-status \
  --no-create-home --shell /bin/sh --password '*' castle-status
touch /etc/ssh/castle-status.keys && chmod 644 /etc/ssh/castle-status.keys
cat > /etc/ssh/sshd_config.d/00-castle-status.conf <<'EOF'
Match User castle-status
    AuthenticationMethods publickey
    PasswordAuthentication no
    KbdInteractiveAuthentication no
    AuthorizedKeysFile /etc/ssh/castle-status.keys
    ForceCommand /usr/bin/cat /var/lib/castle-status/status.json
    PermitTTY no
    DisableForwarding yes
    AllowTcpForwarding no
    AllowStreamLocalForwarding no
    AllowAgentForwarding no
    X11Forwarding no
    PermitTunnel no
    PermitUserRC no
    AcceptEnv CASTLE_STATUS_TAKES_NONE
    MaxSessions 1
    UnusedConnectionTimeout 30
EOF
if sshd -t; then systemctl reload ssh; else rm /etc/ssh/sshd_config.d/00-castle-status.conf; fi
uid="$(id -u castle-status)" && [ -n "$uid" ] \
  && mkdir -p "/etc/systemd/system/user-$uid.slice.d" \
  && printf '[Slice]\nTasksMax=12\n' > "/etc/systemd/system/user-$uid.slice.d/castle-status.conf" \
  && systemctl daemon-reload
```

Then see that sshd gives the account what the block says, and nothing that
another block set before it:

```bash
diff <(sshd -T -C user=castle-status,host=x,addr=127.0.0.1 | grep -E '^(authenticationmethods|passwordauthentication|kbdinteractiveauthentication|authorizedkeysfile|forcecommand|permittty|disableforwarding|allowtcpforwarding|allowstreamlocalforwarding|allowagentforwarding|x11forwarding|permittunnel|permituserrc|acceptenv|maxsessions|unusedconnectiontimeout) ' | sort) - <<'EOF' && echo "the block is in effect"
acceptenv CASTLE_STATUS_TAKES_NONE
allowagentforwarding no
allowstreamlocalforwarding no
allowtcpforwarding no
authenticationmethods publickey
authorizedkeysfile /etc/ssh/castle-status.keys
disableforwarding yes
forcecommand /usr/bin/cat /var/lib/castle-status/status.json
kbdinteractiveauthentication no
maxsessions 1
passwordauthentication no
permittty no
permittunnel no
permituserrc no
unusedconnectiontimeout 30
x11forwarding no
EOF
```

It prints `the block is in effect` and nothing else. A line it prints instead
is a setting that is not as the block has it:

- `forcecommand none` and the rest at their defaults: sshd refused the block,
  and it was removed again rather than left for the next restart to stumble
  over; or the main `sshd_config` does not include
  `/etc/ssh/sshd_config.d/*.conf`.
- single lines that differ: another `Match` block that also fits this account
  is read before this one, and sshd keeps the first value it meets for each
  setting. The file's name puts it first among the drop-ins; a block in the
  main file above its `Include` line, or a drop-in whose name sorts before
  `00-castle-status.conf`, has to make way.

The account signs in with a key and in no other way: its password field is
one that no password matches. Whatever the other side asks for, the session
runs that one command: no terminal, no forwarding, no other program, and no
environment of the other side's choosing (`AcceptEnv` names a variable that
nobody sets, in place of the ones the main file takes). A connection that asks
for nothing is closed after half a minute. The shell is `/bin/sh` because sshd
starts the command through the account's shell. Its keys are in a file of
root's, so the account cannot give itself another, and it can write nowhere in
its directory.

Everything the account has running at one time is held to twelve processes,
which is two answers under way: systemd places each of its ssh sessions in the
account's slice (on Debian, `libpam-systemd` does that), and the block's last
lines put that limit on the slice of this account and of no other: they do
nothing when the account is not there. Someone who has a key and keeps
connections open, reading nothing, can hold that much and no more; while they
do, the next connection is refused, and the overview shows the server as one
that gives no status.

### Giving the overview a key

The machine that runs the overview makes a key
([`../../overview/README.md`](../../overview/README.md)). Its public half goes
on one line of the keys file, with `restrict` in front:

```bash
echo 'restrict ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIexamplekeyexamplekeyexamplekeyexamplekeyex overview' >> /etc/ssh/castle-status.keys
```

That machine also needs this server's host key, to know that it is asking the
right server. Read it here, where it cannot have been swapped on the way:

```bash
cat /etc/ssh/ssh_host_ed25519_key.pub
```

### Taking a key away

Delete its line from `/etc/ssh/castle-status.keys`. The next connection with
that key is refused; nothing has to be reloaded.

To stop publishing the status altogether (this can be run again as well, and
takes away only what the steps above made):

```bash
systemctl disable --now castle-status.timer
rm -f /etc/systemd/system/castle-status.{service,timer}
for d in /etc/systemd/system/user-*.slice.d; do
  [ -e "$d/castle-status.conf" ] && rm "$d/castle-status.conf" && rmdir --ignore-fail-on-non-empty "$d"
done
systemctl daemon-reload
rm -f /etc/ssh/sshd_config.d/00-castle-status.conf /etc/ssh/castle-status.keys
sshd -t && systemctl reload ssh
loginctl terminate-user castle-status 2>/dev/null || true   # what is left of its sessions
sleep 2
getent passwd castle-status >/dev/null && userdel castle-status   # takes its group with it
getent group castle-status >/dev/null && groupdel castle-status
rm -rf /var/lib/castle-status
```
