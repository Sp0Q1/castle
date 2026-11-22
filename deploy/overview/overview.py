#!/usr/bin/env python3
"""One page that says how every castle server is.

Reads a configuration, asks each server in it for its status over ssh
(deploy/compose/docs/status.md) and writes one static page. A server's answer
is taken for what it is: one that is not exactly a status is shown as not
recognised, and nothing of it reaches the page.
"""

import argparse
import base64
import datetime
import hashlib
import html
import json
import os
import re
import resource
import selectors
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib

UTC = datetime.timezone.utc
SSH = ["ssh"]
ANSWER_BYTES = 1 << 20
ANSWER_SECONDS = 30
# The most ssh may write to the file it keeps its own diagnostics in.
DIAGNOSTIC_BYTES = 1 << 16
# From when a server's part of the page says that it needs a look.
STALE_AFTER = datetime.timedelta(minutes=15)
CERTIFICATE_LEFT = datetime.timedelta(days=21)
BACKUP_AGE = datetime.timedelta(hours=36)
DISK_PERCENT = 85
NEWS_SHOWN = 30

SEVERITIES = ("extreme", "high", "elevated", "medium", "low")
ROLES = ("client", "staff", "manager")
HEALTH = ("healthy", "starting", "unhealthy")
KINDS = ("tenant", "canary", "cert-only")
SERVICES = ("caddy", "keycloak", "keycloak-db", "dns")
EVERYWHERE = frozenset(SERVICES[:3])
INSTANT = re.compile(r"20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z")
CODENAME = re.compile(r"[a-z][a-z0-9-]{1,30}")
DIGEST = re.compile(r"[0-9a-f]{12}")


# --- what a status is -------------------------------------------------------
# Each function takes a value as JSON gave it and returns it as the page may
# use it, or raises. Nothing is passed on that was not looked at.


class Unrecognised(ValueError):
    """The answer is not a status."""


def need(holds, what):
    if not holds:
        raise Unrecognised(what)


def fields(value, names, what):
    need(type(value) is dict and value.keys() == set(names), f"{what}: not the fields it has")
    return value


def whole(value, low, high, what):
    integral = type(value) is int or (type(value) is float and value.is_integer())
    need(integral and low <= value <= high, f"{what}: not a whole number from {low} to {high}")
    return int(value)


def instant(value, what):
    need(type(value) is str and INSTANT.fullmatch(value) is not None, f"{what}: not a time")
    try:
        return datetime.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=UTC)
    except ValueError:
        raise Unrecognised(f"{what}: not a time") from None


def word(value, words, what):
    need(type(value) is str and value in words, f"{what}: not one of its words")
    return value


def shaped(value, shape, what):
    need(type(value) is str and shape.fullmatch(value) is not None, f"{what}: not its shape")
    return value


def flag(value, what):
    need(type(value) is bool, f"{what}: not true or false")
    return value


def version(value, what):
    need(type(value) in (int, float) and value == 1, f"{what}: not version 1")
    return 1


def optional(value, known, *rest):
    """None is what a server says when it could not tell."""
    return None if value is None else known(value, *rest)


def listed(value, most, one, what):
    need(type(value) is list and len(value) <= most, f"{what}: not a list of at most {most}")
    return [one(item) for item in value]


def distinct(items, key, what):
    need(len({item[key] for item in items}) == len(items), f"{what}: one of them twice")
    return items


def tally(value, names, what):
    value = fields(value, names, what)
    return {name: whole(value[name], 0, 1_000_000, what) for name in names}


def project(value):
    value = fields(
        value, ("id", "drafts", "published", "by_severity", "comments", "last_activity"), "project"
    )
    return {
        "id": whole(value["id"], 1, 10**12, "project id"),
        "drafts": whole(value["drafts"], 0, 1_000_000, "drafts"),
        "published": whole(value["published"], 0, 1_000_000, "published"),
        "by_severity": tally(value["by_severity"], SEVERITIES, "severities"),
        "comments": tally(value["comments"], ROLES, "comments"),
        "last_activity": optional(value["last_activity"], instant, "last activity"),
    }


def summary(value):
    """What a castle says about itself: src/tasks/summary.rs, deploy/compose/summary.jq."""
    value = fields(
        value, ("summary_version", "generated_at", "window_days", "projects", "activity"), "summary"
    )
    projects = distinct(listed(value["projects"], 500, project, "projects"), "id", "projects")
    ids = {one["id"] for one in projects}

    def comment(item):
        item = fields(item, ("at", "project_id", "finding_id", "by"), "comment")
        project_id = whole(item["project_id"], 1, 10**12, "comment's project")
        need(project_id in ids, "comment: on a project that is not listed")
        return {
            "at": instant(item["at"], "comment's time"),
            "project_id": project_id,
            "finding_id": whole(item["finding_id"], 1, 10**12, "finding id"),
            "by": word(item["by"], ROLES, "comment's author"),
        }

    return {
        "summary_version": version(value["summary_version"], "summary"),
        "generated_at": instant(value["generated_at"], "summary's time"),
        "window_days": whole(value["window_days"], 1, 90, "window"),
        "projects": projects,
        "activity": listed(value["activity"], 50, comment, "activity"),
    }


def instance(value):
    value = fields(
        value, ("codename", "kind", "health", "image", "certificate_ends", "summary"), "instance"
    )
    one = {
        "codename": shaped(value["codename"], CODENAME, "codename"),
        "kind": word(value["kind"], KINDS, "kind"),
        "health": optional(value["health"], word, HEALTH, "health"),
        "image": optional(value["image"], shaped, DIGEST, "image"),
        "certificate_ends": optional(value["certificate_ends"], instant, "certificate"),
        "summary": optional(value["summary"], summary),
    }
    need(
        one["kind"] == "tenant" or (one["image"] is None and one["summary"] is None),
        "instance: has what only a tenant has",
    )
    need(
        one["kind"] != "cert-only" or (one["health"] is None and one["certificate_ends"] is None),
        "instance: has what only a running one has",
    )
    return one


def service(value):
    value = fields(value, ("service", "health"), "service")
    return {
        "service": word(value["service"], SERVICES, "service"),
        "health": optional(value["health"], word, HEALTH, "health"),
    }


def status(value):
    """What a server says about itself: `castlectl status`."""
    value = fields(
        value,
        (
            "status_version",
            "generated_at",
            "platform",
            "sign_in_certificate_ends",
            "instances",
            "newest_backup",
            "host",
        ),
        "status",
    )
    host = fields(
        value["host"],
        ("reboot_required", "failed_units", "disk_used_percent", "renewal_timer", "backup_timer"),
        "host",
    )
    platform = distinct(
        listed(value["platform"], len(SERVICES), service, "platform"), "service", "platform"
    )
    need(EVERYWHERE <= {one["service"] for one in platform}, "platform: not every service a server has")
    return {
        "status_version": version(value["status_version"], "status"),
        "generated_at": instant(value["generated_at"], "status's time"),
        "platform": platform,
        "sign_in_certificate_ends": optional(
            value["sign_in_certificate_ends"], instant, "sign-in certificate"
        ),
        "instances": distinct(
            listed(value["instances"], 500, instance, "instances"), "codename", "instances"
        ),
        "newest_backup": optional(value["newest_backup"], instant, "newest backup"),
        "host": {
            "reboot_required": optional(host["reboot_required"], flag, "reboot required"),
            "failed_units": optional(host["failed_units"], whole, 0, 100_000, "failed units"),
            "disk_used_percent": optional(host["disk_used_percent"], whole, 0, 100, "disk"),
            "renewal_timer": optional(host["renewal_timer"], flag, "renewal timer"),
            "backup_timer": optional(host["backup_timer"], flag, "backup timer"),
        },
    }


def once(pairs):
    value = dict(pairs)
    need(len(value) == len(pairs), "a field twice")
    return value


def no_constant(name):
    raise Unrecognised(f"{name}: not a number")


def document(answer):
    """The status in the bytes a server answered with."""
    try:
        value = json.loads(
            answer.decode("utf-8"), object_pairs_hook=once, parse_constant=no_constant
        )
    except Unrecognised:
        raise
    except (ValueError, RecursionError):
        raise Unrecognised("not JSON") from None
    return status(value)


# --- asking a server --------------------------------------------------------


class NoAnswer(Exception):
    """Why a server gave no status. The words are this program's, never the server's."""


def held():
    """In the program that is about to be run: no file of more than DIAGNOSTIC_BYTES, and no core."""
    resource.setrlimit(resource.RLIMIT_FSIZE, (DIAGNOSTIC_BYTES, DIAGNOSTIC_BYTES))
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))


def run(argv, most, seconds):
    """Runs a program, not through a shell: its exit status, what it printed and
    the start of what it complained. Stops it when it prints more than `most`
    bytes or takes longer than `seconds`, and the system stops it when it
    writes more than DIAGNOSTIC_BYTES to a file."""
    deadline = time.monotonic() + seconds
    printed, complained = bytearray(), bytearray()
    with subprocess.Popen(
        argv,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LC_ALL": "C"},
        preexec_fn=held,
    ) as process, selectors.DefaultSelector() as streams:
        streams.register(process.stdout, selectors.EVENT_READ, printed)
        streams.register(process.stderr, selectors.EVENT_READ, complained)
        try:
            while streams.get_map():
                left = deadline - time.monotonic()
                if left <= 0:
                    raise NoAnswer("no answer in time")
                for key, _ in streams.select(left):
                    chunk = os.read(key.fd, 65536)
                    if not chunk:
                        streams.unregister(key.fileobj)
                    elif key.data is printed:
                        printed += chunk
                        if len(printed) > most:
                            raise NoAnswer("an answer that is too large")
                    else:
                        complained += chunk[: 4096 - len(complained)]
            return process.wait(max(deadline - time.monotonic(), 1)), bytes(printed), bytes(complained)
        except subprocess.TimeoutExpired:
            raise NoAnswer("no answer in time") from None
        finally:
            process.kill()


def ask(server):
    """The bytes a server's status account answers with.

    Why there are none is read from what ssh itself says when it gives up,
    which it writes to a file of its own (-E), apart from anything the far
    side prints. It is told to write nothing else there (LogLevel=FATAL)."""
    handle, said = tempfile.mkstemp(prefix="overview-ssh-")
    os.close(handle)
    try:
        return asked(server, said)
    finally:
        os.unlink(said)


def asked(server, said):
    exit_status, printed, _ = run(
        [
            *SSH,
            "-F", "none",
            "-E", said,
            "-T",
            "-o", "BatchMode=yes",
            "-o", "StrictHostKeyChecking=yes",
            "-o", f"UserKnownHostsFile={server['known_hosts']}",
            "-o", "GlobalKnownHostsFile=/dev/null",
            "-o", "UpdateHostKeys=no",
            "-o", "IdentitiesOnly=yes",
            "-o", "IdentityAgent=none",
            "-o", f"IdentityFile={server['key']}",
            "-o", "PreferredAuthentications=publickey",
            "-o", "ClearAllForwardings=yes",
            "-o", "ForwardAgent=no",
            "-o", "PermitLocalCommand=no",
            "-o", "ConnectTimeout=10",
            "-o", "ServerAliveInterval=5",
            "-o", "ServerAliveCountMax=3",
            "-o", "LogLevel=FATAL",
            "-p", str(server["port"]),
            "-l", server["user"],
            "--", server["host"],
        ],
        ANSWER_BYTES,
        ANSWER_SECONDS,
    )
    if exit_status == 0:
        return printed
    if 0 < exit_status < 255:
        raise NoAnswer("the server has none to give yet, or could not read it")
    with open(said, "rb") as diagnostics:
        told = diagnostics.read(DIAGNOSTIC_BYTES + 1)
    # The far side can put words of its own into some of what ssh writes, line
    # ends included, but not in front of it. So the host key is the reason only
    # when ssh's one line about it is all there is, and a refused key only when
    # ssh's line about that, which names the host in lower case, comes first.
    if told in (b"Host key verification failed.\r\n", b"Host key verification failed.\n"):
        raise NoAnswer("another host key than the one pinned here")
    if told.startswith(f"{server['user']}@{server['host'].lower()}: Permission denied (".encode()):
        raise NoAnswer("the server does not take this key")
    raise NoAnswer("no connection")


# --- the configuration ------------------------------------------------------


class Misconfigured(Exception):
    pass


SETTINGS = {
    "name": r"[^\0-\x1f\x7f]{1,40}",
    "host": r"[0-9A-Za-z][0-9A-Za-z.:-]{0,252}",
    "user": r"[a-z_][a-z0-9_-]{0,31}",
    "key": r"/[A-Za-z0-9._/+@-]+",
    "known_hosts": r"/[A-Za-z0-9._/+@-]+",
    "base_domain": r"[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)+",
}


def configuration(path):
    """The servers to ask and where the page goes, or Misconfigured."""

    def want(holds, what):
        if not holds:
            raise Misconfigured(f"{path}: {what}")

    try:
        with open(path, "rb") as source:
            closed = os.fstat(source.fileno()).st_mode & 0o077 == 0
            given = tomllib.load(source)
    except RecursionError:
        raise Misconfigured(f"{path}: tables or arrays within one another, deeper than can be read") from None
    except (OSError, ValueError) as error:
        raise Misconfigured(f"{path}: {error}") from None
    want(closed, "is open to others, and it says which codenames are your clients: chmod 600")
    want(given.keys() == {"output", "servers"}, "wants `output` and `[[servers]]`, and nothing else")
    output = given["output"]
    want(
        type(output) is str
        and "\0" not in output
        and os.path.isabs(output)
        and os.path.isdir(os.path.dirname(output))
        and not os.path.isdir(output),
        "`output` is not a file's path in a directory that is there",
    )
    want(os.access(os.path.dirname(output), os.W_OK | os.X_OK), "the directory of `output` cannot be written to")
    tables = given["servers"]
    want(
        type(tables) is list and tables and all(type(table) is dict for table in tables),
        "wants at least one `[[servers]]`",
    )

    servers = []
    for at, table in enumerate(tables, start=1):
        where = f"server {at}"
        server = {"port": 22, "clients": {}} | table
        known = SETTINGS.keys() | {"port", "clients"}
        want(server.keys() == known, f"{where}: wants {', '.join(SETTINGS)}")
        for name, shape in SETTINGS.items():
            fits = type(server[name]) is str and re.fullmatch(shape, server[name]) is not None
            want(fits, f"{where}: `{name}` is not what it should be")
        for name in ("key", "known_hosts"):
            want(os.path.isfile(server[name]), f"{where}: `{name}` is not a file: {server[name]}")
        closed = os.stat(server["key"]).st_mode & 0o077 == 0
        want(closed, f"{where}: `key` is open to others, and ssh refuses such a key")
        want(type(server["port"]) is int and 0 < server["port"] < 65536, f"{where}: `port` is not a port")
        want(type(server["clients"]) is dict, f"{where}: `clients` is not a table")
        for codename, name in server["clients"].items():
            want(CODENAME.fullmatch(codename) is not None, f"{where}: `{codename}` is not a codename")
            named = type(name) is str and 0 < len(name) <= 80 and name.isprintable()
            want(named, f"{where}: what `{codename}` is called is not a line of text")
        servers.append(server)
    want(len({server["name"] for server in servers}) == len(servers), "two servers with one name")
    return {"output": output, "servers": servers}


# --- the page ---------------------------------------------------------------


class Markup(str):
    """Text that is HTML already. Everything else is escaped where it is used."""


def opening(tag, attributes):
    return tag + "".join(
        f' {name.rstrip("_").replace("_", "-")}="{html.escape(str(value), quote=True)}"'
        for name, value in attributes.items()
        if value is not None
    )


def el(tag, *children, **attributes):
    inside = "".join(
        child if isinstance(child, Markup) else html.escape(str(child), quote=True)
        for child in children
    )
    return Markup(f"<{opening(tag, attributes)}>{inside}</{tag}>")


def meta(**attributes):
    return Markup(f"<{opening('meta', attributes)}>")


def join(parts, between=", "):
    out = []
    for part in parts:
        if out:
            out.append(between)
        out.append(part)
    return el("span", *out)


STYLE = """
:root { --bg: #16191d; --surface: #1e2329; --border: #2f363d; --text: #dee2e6; --muted: #93a1ad;
        --primary: #4c8bf5; --alert: #ff9ea1; --fine: #7ee196; }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text);
       font: 15px/1.55 -apple-system, "Segoe UI", Roboto, Arial, sans-serif; }
.wrap { max-width: 1100px; margin: 0 auto; padding: 0 20px; }
.bar { background: var(--surface); border-bottom: 1px solid var(--border); }
.bar .wrap { display: flex; align-items: center; justify-content: space-between; min-height: 60px; }
main { padding-bottom: 40px; }
h2 { font-size: 1.05rem; margin: 28px 0 10px; }
h3 { font-size: 1rem; margin: 0 0 4px; }
a { color: var(--primary); text-decoration: none; }
a:hover { text-decoration: underline; }
code { font: 13px ui-monospace, monospace; color: inherit; }
ul { margin: 4px 0; padding-left: 1.2rem; }
.muted { color: var(--muted); }
.small { font-size: 0.82rem; }
.alert, .down { color: var(--alert); }
.fine { color: var(--fine); }
.note { margin: 18px 0 0; padding: 10px 14px; border: 1px solid var(--border); border-radius: 8px;
        background: var(--surface); }
.look { border-color: var(--alert); color: var(--alert); }
table { width: 100%; border-collapse: collapse; background: var(--surface); border: 1px solid var(--border);
        border-radius: 10px; overflow: hidden; }
th, td { text-align: left; padding: 10px 12px; border-bottom: 1px solid var(--border); vertical-align: top; }
th { color: var(--muted); font-weight: 600; font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.04em; }
tr:last-child td { border-bottom: 0; }
td:last-child, .tight { white-space: nowrap; }
tr.stale td { opacity: 0.55; }
.badge { display: inline-block; margin: 0 6px 4px 0; padding: 1px 9px; border-radius: 999px; font-size: 0.75rem;
         font-weight: 700; background: #2a3038; color: var(--text); white-space: nowrap; }
.badge.new { background: rgba(76, 139, 245, 0.2); color: #9cc0ff; }
.sev-extreme { background: rgba(255, 107, 107, 0.2); color: #ff6b6b; }
.sev-high { background: rgba(255, 143, 107, 0.2); color: #ff8f6b; }
.sev-elevated { background: rgba(245, 172, 107, 0.2); color: #f5ac6b; }
.sev-medium { background: rgba(240, 207, 107, 0.2); color: #f0cf6b; }
.sev-low { background: rgba(126, 225, 150, 0.2); color: #7ee196; }
.btn { display: inline-block; padding: 5px 12px; border-radius: 7px; background: var(--primary); color: #fff;
       font-weight: 600; font-size: 0.85rem; }
.btn:hover { text-decoration: none; }
.activity { list-style: none; margin: 0; padding: 0; background: var(--surface); border: 1px solid var(--border);
            border-radius: 10px; }
.activity li { display: flex; gap: 14px; padding: 10px 14px; border-bottom: 1px solid var(--border); }
.activity li:last-child { border-bottom: 0; }
.activity .when { flex: none; width: 90px; color: var(--muted); }
.activity .from-client { border-left: 3px solid var(--primary); }
.server { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 12px 14px;
          margin: 0 0 12px; }
.server p { margin: 4px 0; }
.server.stale > :not(h3):not(.alert) { opacity: 0.55; }
"""


def when(at):
    return at.strftime("%Y-%m-%d %H:%M UTC")


def lasting(delta):
    minutes = int(abs(delta).total_seconds() // 60)
    if minutes < 120:
        return f"{minutes} min"
    if minutes < 36 * 60:
        return f"{minutes // 60} h"
    return f"{minutes // 1440} days"


def ago(at, now):
    """`3 h ago`, as of when the page was made, with the time itself to hover over."""
    apart = now - at
    if abs(apart) < datetime.timedelta(seconds=90):
        words = "just now"
    else:
        words = f"{lasting(apart)} {'ago' if apart > datetime.timedelta(0) else 'ahead'}"
    return el("span", words, title=when(at))


def portal(server, codename, path=""):
    return f"https://{codename}.{server['base_domain']}/{path}"


def link(text, to, class_=None):
    return el("a", text, href=to, rel="noreferrer", class_=class_)


def health(value):
    if value is None:
        return el("span", "not known", class_="alert")
    return el("span", value, class_="fine" if value == "healthy" else "alert")


def badges(*counts):
    """A badge for every count that is not zero, each given as (count, words, kind); or `none`."""
    shown = [el("span", words, class_=f"badge {kind}".strip()) for count, words, kind in counts if count]
    return shown or [el("span", "none", class_="muted")]


def concerns(server, said, now):
    """What on a server needs a look, in words."""
    found = []

    def state(what, value):
        if value is None:
            found.append(f"the state of {what} could not be read")
        elif value != "healthy":
            found.append(f"{what} is {value}")

    for one in said["platform"]:
        state(one["service"], one["health"])
    served = [("the sign-in host", said["sign_in_certificate_ends"])]
    for one in said["instances"]:
        if one["kind"] == "cert-only":
            continue
        state(one["codename"], one["health"])
        served.append((one["codename"], one["certificate_ends"]))
        if one["kind"] != "tenant":
            continue
        if one["codename"] not in server["clients"]:
            found.append(f"{one['codename']} is a tenant on the server and has no name here")
        if one["image"] is None:
            found.append(f"what {one['codename']} runs could not be read")
        if one["summary"] is None:
            found.append(f"{one['codename']} gave no summary, or it was left out for size")
        elif abs(said["generated_at"] - one["summary"]["generated_at"]) > STALE_AFTER:
            found.append(f"the summary of {one['codename']} is of {when(one['summary']['generated_at'])}")
    for whose, end in served:
        if end is None:
            found.append(f"the certificate of {whose} could not be read")
        elif end - now < CERTIFICATE_LEFT:
            found.append(f"the certificate of {whose} ends {when(end)}")

    tenants = {one["codename"] for one in said["instances"] if one["kind"] == "tenant"}
    for codename in sorted(server["clients"].keys() - tenants):
        found.append(f"{codename} has a name here, and the server lists no such tenant")

    newest, host = said["newest_backup"], said["host"]
    if newest is None:
        found.append("no backup was found")
    elif now - newest > BACKUP_AGE:
        found.append(f"the newest backup is of {when(newest)}")
    if host["disk_used_percent"] is None:
        found.append("how full the disk is could not be read")
    elif host["disk_used_percent"] >= DISK_PERCENT:
        found.append(f"the disk is {host['disk_used_percent']} percent full")
    if host["reboot_required"] is None:
        found.append("whether a restart is waiting could not be read")
    elif host["reboot_required"]:
        found.append("a restart is waiting")
    if host["failed_units"] is None:
        found.append("whether units failed could not be read")
    elif host["failed_units"]:
        found.append(f"{host['failed_units']} unit(s) failed")
    for timer, what in (("renewal_timer", "certificate renewal"), ("backup_timer", "backup")):
        if host[timer] is None:
            found.append(f"the {what} timer could not be read")
        elif not host[timer]:
            found.append(f"the {what} timer is not active")
    return found


def client_rows(server, said, now, stale):
    """The rows of the clients table for one server: a row for each project of each tenant."""
    rows = []
    for one in said["instances"]:
        if one["kind"] != "tenant":
            continue
        codename, summary = one["codename"], one["summary"]
        client = [
            el("strong", server["clients"].get(codename) or codename),
            el("div", link(el("code", codename), portal(server, codename)), " · ", server["name"], class_="muted small"),
        ]
        if one["health"] != "healthy":
            client.append(el("div", health(one["health"]), class_="small"))
        dim = "stale" if stale else None
        if summary is None:
            said = "Its castle gave no summary that could be used, or it was left out for size."
            words = el("td", said, colspan=6, class_="down")
            rows.append(el("tr", el("td", *client), words, class_=dim))
            continue
        if not summary["projects"]:
            rows.append(el("tr", el("td", *client), el("td", "No project yet.", colspan=6, class_="muted"), class_=dim))
        for at, each in enumerate(summary["projects"]):
            theirs = each["comments"]["client"]
            ours = each["comments"]["staff"] + each["comments"]["manager"]
            last = each["last_activity"]
            rows.append(
                el(
                    "tr",
                    el("td", *client, rowspan=len(summary["projects"])) if at == 0 else "",
                    el("td", f"Project {each['id']}", class_="tight"),
                    el("td", *badges(*((each["by_severity"][s], f"{each['by_severity'][s]} {s}", f"sev-{s}") for s in SEVERITIES))),
                    el("td", each["drafts"]),
                    el(
                        "td",
                        *badges((theirs, f"{theirs} from the client", "new"), (ours, f"{ours} from us", "")),
                        title=f"in the last {summary['window_days']} days",
                    ),
                    el("td", ago(last, now) if last else el("span", "none", class_="muted")),
                    el("td", link("Open", portal(server, codename, f"projects/{each['id']}"), class_="btn")),
                    class_=dim,
                )
            )
    return rows


def unknown_rows(server):
    """The clients this configuration names on a server that gave no status."""
    return [
        el(
            "tr",
            el("td", el("strong", name), el("div", el("code", codename), " · ", server["name"], class_="muted small")),
            el("td", f"Not known: {server['name']} gave no status.", colspan=6, class_="down"),
        )
        for codename, name in sorted(server["clients"].items())
    ]


def news(results, now):
    """The newest comments on every server that gave a status, newest first."""
    comments = [
        (each["at"], server, one["codename"], each)
        for server, outcome in results
        if not isinstance(outcome, Exception)
        for one in outcome["instances"]
        if one["kind"] == "tenant" and one["summary"]
        for each in one["summary"]["activity"]
    ]
    comments.sort(key=lambda comment: comment[0], reverse=True)
    return [
        el(
            "li",
            el("span", ago(at, now), class_="when"),
            el(
                "span",
                el("strong", server["clients"].get(codename) or codename),
                " · The client commented on " if each["by"] == "client" else " · We commented on ",
                link(f"finding {each['finding_id']}", portal(server, codename, f"findings/{each['finding_id']}")),
                f" in project {each['project_id']}",
            ),
            class_="from-client" if each["by"] == "client" else None,
        )
        for at, server, codename, each in comments[:NEWS_SHOWN]
    ]


def running(said):
    """What each tenant runs and until when its certificate is good."""
    tenants = [one for one in said["instances"] if one["kind"] == "tenant"]
    if not tenants:
        return ""
    return el(
        "p",
        "Tenants: ",
        join(
            el(
                "span",
                el("code", one["codename"]),
                " ",
                health(one["health"]),
                f" image {one['image'] or 'not known'}, certificate until "
                f"{when(one['certificate_ends']) if one['certificate_ends'] else 'not known'}",
            )
            for one in tenants
        ),
        class_="muted",
    )


def decoys(said):
    others = [one for one in said["instances"] if one["kind"] != "tenant"]
    if not others:
        return ""
    canaries = sum(one["kind"] == "canary" for one in others)
    return el(
        "p",
        f"{canaries} canaries, {len(others) - canaries} with a certificate only: ",
        join(
            el(
                "span",
                el("code", one["codename"]),
                " ",
                health(one["health"]) if one["kind"] == "canary" else "certificate only",
            )
            for one in others
        ),
        class_="muted",
    )


def facts(said):
    newest, used = said["newest_backup"], said["host"]["disk_used_percent"]
    return el(
        "p",
        join(el("span", one["service"], " ", health(one["health"])) for one in said["platform"]),
        f". Newest backup: {when(newest) if newest else 'none'}."
        f" Disk: {'not known' if used is None else f'{used} percent'}.",
        class_="muted",
    )


def card(server, outcome, now):
    """A server's own part of the page, what on it needs a look, and whether what it said is of now."""
    title = el("h3", server["name"])
    if isinstance(outcome, (NoAnswer, Unrecognised)):
        words = (
            f"No status: {outcome}."
            if isinstance(outcome, NoAnswer)
            else "The server answered with something that is not a status. Nothing of it is shown."
        )
        return el("div", title, el("p", words, class_="alert"), class_="server stale"), [words], True
    said = outcome
    written = when(said["generated_at"])
    age = now - said["generated_at"]
    found = concerns(server, said, now)
    if age > STALE_AFTER:
        found.insert(
            0,
            f"Old: the newest status this server has is of {written}, {lasting(age)} before this page was made."
            " What is shown is how it was then.",
        )
    elif -age > STALE_AFTER:
        found.insert(
            0,
            f"A clock is wrong: this status is of {written}, {lasting(age)} after this page was made."
            " What is shown may not be how it is now.",
        )
    stale = abs(age) > STALE_AFTER
    look = (
        el("ul", *(el("li", each) for each in found), class_="alert")
        if found
        else el("p", "Nothing needs a look.", class_="fine")
    )
    parts = [title, el("p", f"Status of {written}.", class_="muted"), look, facts(said), running(said), decoys(said)]
    return el("div", *parts, class_="server stale" if stale else "server"), found, stale


def page(results, now):
    cards, found, rows = [], [], []
    for server, outcome in results:
        made, concerning, stale = card(server, outcome, now)
        cards.append(made)
        found += [(server["name"], words) for words in concerning]
        if isinstance(outcome, (NoAnswer, Unrecognised)):
            rows += unknown_rows(server)
        else:
            rows += client_rows(server, outcome, now, stale)
    looking = len({name for name, _ in found})
    digest = base64.b64encode(hashlib.sha256(STYLE.encode()).digest()).decode()
    head = el(
        "head",
        meta(charset="utf-8"),
        meta(
            http_equiv="Content-Security-Policy",
            content=f"default-src 'none'; style-src 'sha256-{digest}'; base-uri 'none'; form-action 'none'",
        ),
        meta(name="referrer", content="no-referrer"),
        meta(http_equiv="refresh", content="60"),
        meta(name="viewport", content="width=device-width, initial-scale=1"),
        el("title", "All clients"),
        el("style", Markup(STYLE)),
    )
    columns = ("Client", "Project", "Published, by severity", "Drafts", "New comments", "Last activity", "")
    body = el(
        "body",
        el(
            "header",
            el(
                "div",
                el("strong", "All clients"),
                el(
                    "span",
                    f"Made {when(now)} · {len(results)} server(s), {looking or 'nothing'} to look at",
                    class_="muted",
                ),
                class_="wrap",
            ),
            class_="bar",
        ),
        el(
            "main",
            el(
                "div",
                el("strong", "Needs a look"),
                el("ul", *(el("li", f"{name}: {words}") for name, words in found)),
                class_="note look",
            )
            if found
            else "",
            el("h2", "Clients"),
            el(
                "table",
                el("thead", el("tr", *(el("th", column) for column in columns))),
                el("tbody", *(rows or [el("tr", el("td", "No client yet.", colspan=7, class_="muted"))])),
            ),
            el("h2", "What is new"),
            el(
                "ul",
                *(news(results, now) or [el("li", "No comment in the last days.", class_="muted")]),
                class_="activity",
            ),
            el("h2", "Servers"),
            *cards,
            el(
                "p",
                "No titles, descriptions, comment texts or credentials are held here. Every link opens the client's own"
                " portal, where you sign in. The page cannot tell the time: if it was made long ago, nothing is making"
                " it any more.",
                class_="muted small",
            ),
            class_="wrap",
        ),
    )
    return "<!doctype html>\n" + el("html", head, body, lang="en") + "\n"


def collect(server):
    try:
        return document(ask(server))
    except (NoAnswer, Unrecognised) as outcome:
        return outcome


def write(path, text):
    directory = os.path.dirname(path)
    handle, beside = tempfile.mkstemp(dir=directory, prefix=".overview-")
    try:
        with os.fdopen(handle, "w", encoding="utf-8") as out:
            out.write(text)
        os.replace(beside, path)
    except BaseException:
        os.unlink(beside)
        raise


def main():
    arguments = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    arguments.add_argument(
        "--config",
        default=os.path.expanduser("~/.config/castle-overview/overview.toml"),
        help="the configuration (default: %(default)s)",
    )
    given = arguments.parse_args()
    try:
        configured = configuration(given.config)
    except Misconfigured as error:
        sys.exit(f"overview: {error}")
    if shutil.which(SSH[0]) is None:
        sys.exit(f"overview: there is no `{SSH[0]}` to ask the servers with")
    results = [(server, collect(server)) for server in configured["servers"]]
    write(configured["output"], page(results, datetime.datetime.now(UTC).replace(microsecond=0)))


if __name__ == "__main__":
    main()
