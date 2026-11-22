# The overview

One page that says how every castle server is: which instances run and how
they are, where a client has written something new, and what on a server needs
a look. It is made on a machine of yours, from what each server says about
itself ([`../compose/docs/status.md`](../compose/docs/status.md)).

It only reads. The machine asks each server over ssh, with a key that can read
that server's status and do nothing else. Nothing listens anywhere for it, on
the servers or on the machine, and nothing on the page can change a server. A
link on the page goes to the client's own portal, where you sign in as always.

A status carries ids, counts and times. What a finding or a comment says stays
on its server. Which client is behind a codename is written in one place, the
configuration on this machine.

## What it needs

A Linux machine of your own, such as a virtual machine on your laptop, with an
encrypted disk: the page and the key together say which codenames are your
clients. On it: `python3` (3.11 or newer), `openssh-client`, a browser, and
this directory.

## Setting it up

1. Make a key. It has no passphrase, because a timer uses it; all it is good
   for is reading a status.

   ```bash
   ssh-keygen -t ed25519 -N '' -C overview -f ~/.ssh/castle-status
   ```

2. On each server, set up the status and its account, and add
   `~/.ssh/castle-status.pub` as a key
   ([`../compose/docs/status.md`](../compose/docs/status.md)).

3. Pin each server's host key. Read it on the server
   (`cat /etc/ssh/ssh_host_ed25519_key.pub`) and write it here behind the
   server's address, one line for each server:

   ```bash
   mkdir -p ~/.config/castle-overview ~/overview
   echo '203.0.113.10 ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIexamplehostkeyexamplehostkeyexamplehostkeyexa' >> ~/.config/castle-overview/known_hosts
   ```

   For a port other than 22 the address is written `[203.0.113.10]:2222`. The
   overview asks a server only when the key it shows is the one written here.

4. Copy [`overview.example.toml`](overview.example.toml) to
   `~/.config/castle-overview/overview.toml`, for you alone, and fill it in:
   where the page goes, each server, and what you call the client behind each
   codename.

   ```bash
   install -m 600 overview.example.toml ~/.config/castle-overview/overview.toml
   ```

   The overview refuses a configuration that others on the machine can read,
   as ssh refuses such a key.

5. Make the page once, and open it:

   ```bash
   python3 overview.py
   xdg-open ~/overview/index.html
   ```

6. Have it made every five minutes. In this directory:

   ```bash
   mkdir -p ~/.config/systemd/user
   cat > ~/.config/systemd/user/castle-overview.service <<EOF
   [Unit]
   Description=Castle overview

   [Service]
   Type=oneshot
   ExecStart=/usr/bin/python3 $PWD/overview.py
   EOF
   cat > ~/.config/systemd/user/castle-overview.timer <<'EOF'
   [Unit]
   Description=Castle overview, every five minutes

   [Timer]
   OnCalendar=*:0/5
   Persistent=true

   [Install]
   WantedBy=timers.target
   EOF
   systemctl --user daemon-reload
   systemctl --user enable --now castle-overview.timer
   ```

## Reading the page

The page has three parts.

- **Clients:** a row for every project of every client, whichever server it is
  on: the published findings by severity, the drafts, the comments of the last
  seven days (the client's, and yours), when something last happened, and a
  button into that project. The codename is a link to the client's portal.
- **What is new:** the newest thirty comments on all servers, newest first,
  the clients' marked, each with a link to its finding.
- **Servers:** for each server its platform, its newest backup, its disk, what
  each tenant runs and until when its certificate is good, and its decoys.

When something needs a look, a box above all of that says what and on which
server: an instance that is not healthy, a certificate that ends within three
weeks, no backup for a day and a half, a disk that is filling up, a timer that
is not active, a value the server could not read, a tenant that has no name in
the configuration, a summary made at another time than its status. A time is
said as it was when the page was made ("3 h ago"); point at it for the time
itself.

| The page says | It means |
|---------------|----------|
| `No status: no connection` | the server could not be reached, or ssh gave up for another reason than the two below |
| `No status: no answer in time` | the server was reached and did not answer within half a minute |
| `No status: an answer that is too large` | more than a megabyte came back. It was not read further |
| `No status: another host key than the one pinned here` | ssh reported a key that is not the one in `known_hosts`, and asks such a server nothing. Find out why before pinning another |
| `No status: the server does not take this key` | the key is not in the server's keys file |
| `No status: the server has none to give yet` | the status timer has not run on the server |
| `something that is not a status` | the answer was not exactly a status. None of it is shown |
| `Old:` | the server's newest status is more than fifteen minutes old: its timer has stopped or fails. What is shown is how it was then, and is dimmed |
| `Not known: … gave no status` | the configuration names this client on a server that gave no status |
| `A clock is wrong` | the status is from more than fifteen minutes ahead of this machine |
| `gave no summary, or it was left out for size` | the tenant's castle did not answer its host with something the host could use; or the server left the summary out to keep its status within what is read here, the largest first, and says how many in its journal (`journalctl -u castle-status`) |
| `has a name here, and the server lists no such tenant` | the configuration names a codename that is not a tenant on that server |
| `is a tenant on the server and has no name here` | the server runs a tenant that the configuration does not name: a client to write down, or a tenant nobody meant to make |
| `what … runs could not be read` | the server could not tell which image the tenant runs |
| `the summary of … is of` | the tenant's castle answered its host with a summary made more than fifteen minutes from the status |

The reason behind a `No status` is taken from what ssh itself says when it
gives up, never from what the far side prints. The far side can put words of
its own into some of that, so the host key is given as the reason only when
ssh's one line about it is all ssh said, and a refused key only when ssh's
line about that comes first. Everything else shows as `no connection`.

The page reloads itself every minute. It has no script and cannot tell the
time: the line at the top says when it was made. If that is long ago, the
timer on this machine is not running.

A server that is taken over can say anything about itself, and can leave
things out. It cannot change what the page shows of another server, put text
or a link of its choosing on the page, or reach this machine: what it answers
is read up to a megabyte, taken apart field by field, and shown only if every
field is what a status has there.

## Tests

```bash
python3 -m unittest discover -s deploy/overview     # from the repository; needs jq
```

`fixtures/` holds summaries and statuses that are taken and ones that are
refused. The summaries are put to `../compose/summary.jq` as well, and the two
have to make the same of every one. `fixtures/summary/taken/castle.json` is
also what castle's own test expects the `summary` task to print.
