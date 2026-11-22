"""What the overview takes for a status, how it asks a server, and what it writes.

Run from the repository: python3 -m unittest discover -s deploy/overview
Needs jq: the summaries in fixtures/ are put to deploy/compose/summary.jq as
well, and the two have to make the same of every one; and what
deploy/compose/status.jq makes of the lines castlectl gathers has to be a
status the overview takes. Needs bash and openssl too: `castlectl status`
itself is run, with stand-ins for the engine and for systemd on its path.
"""

import base64
import copy
import datetime
import hashlib
import html
import html.parser
import json
import os
import pathlib
import random
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest
import unittest.mock

import overview

HERE = pathlib.Path(__file__).parent
FIXTURES = HERE / "fixtures"
COMPOSE = HERE.parent / "compose"


def load(path):
    return json.loads(path.read_text(encoding="utf-8"))


def plain(value):
    """A recognised value as JSON would carry it."""
    if isinstance(value, datetime.datetime):
        return value.strftime("%Y-%m-%dT%H:%M:%SZ")
    if isinstance(value, dict):
        return {key: plain(item) for key, item in value.items()}
    if isinstance(value, list):
        return [plain(item) for item in value]
    return value


def by_jq(value):
    """What the host's recognizer makes of a summary: it rebuilt, or None."""
    done = subprocess.run(
        ["jq", "-c", "-L", str(COMPOSE), 'include "summary"; first(summary) // null'],
        input=json.dumps(value),
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(done.stdout)


def refused(recognise, value):
    try:
        recognise(value)
    except overview.Unrecognised:
        return True
    return False


class Summaries(unittest.TestCase):
    def test_both_recognizers_make_the_same_of_what_they_take(self):
        taken = sorted((FIXTURES / "summary" / "taken").glob("*.json"))
        self.assertIn("castle.json", [path.name for path in taken])
        for path in taken:
            with self.subTest(path.name):
                value = load(path)
                rebuilt = by_jq(value)
                self.assertIsNotNone(rebuilt, "summary.jq refuses it")
                self.assertEqual(plain(overview.summary(value)), rebuilt)

    def test_both_recognizers_refuse_the_rest(self):
        cases = load(FIXTURES / "summary" / "refused.json")
        self.assertGreater(len(cases), 30)
        for name, value in cases.items():
            with self.subTest(name):
                self.assertIsNone(by_jq(value), "summary.jq takes it")
                self.assertTrue(refused(overview.summary, value), "overview.py takes it")

    def test_numbers_written_otherwise_come_out_whole(self):
        value = load(FIXTURES / "summary" / "taken" / "whole-numbers-written-otherwise.json")
        self.assertIs(type(value["projects"][0]["drafts"]), float)
        for rebuilt in (overview.summary(value), by_jq(value)):
            self.assertIs(type(rebuilt["projects"][0]["drafts"]), int)
            self.assertIs(type(rebuilt["summary_version"]), int)

    def test_a_list_longer_than_a_summary_has(self):
        value = load(FIXTURES / "summary" / "taken" / "nothing-yet.json")
        one = load(FIXTURES / "summary" / "taken" / "castle.json")["projects"][1]
        value["projects"] = [dict(one, id=at) for at in range(1, 502)]
        self.assertIsNone(by_jq(value))
        self.assertTrue(refused(overview.summary, value))
        value["projects"].pop()
        self.assertEqual(len(by_jq(value)["projects"]), 500)
        self.assertEqual(plain(overview.summary(value)), by_jq(value))


class Statuses(unittest.TestCase):
    def test_what_castlectl_wrote_is_taken(self):
        taken = sorted((FIXTURES / "status" / "taken").glob("*.json"))
        self.assertIn("a-server.json", [path.name for path in taken])
        for path in taken:
            with self.subTest(path.name):
                self.assertEqual(plain(overview.document(path.read_bytes())), load(path))

    def test_the_rest_is_refused(self):
        cases = load(FIXTURES / "status" / "refused.json")
        self.assertGreater(len(cases), 25)
        for name, value in cases.items():
            with self.subTest(name):
                self.assertTrue(refused(overview.status, value))
                self.assertTrue(refused(overview.document, json.dumps(value).encode()))

    def test_bytes_that_are_not_one_json_document(self):
        good = (FIXTURES / "status" / "taken" / "a-server.json").read_bytes()
        overview.document(good)
        version = b'"status_version": 1,'
        for name, answer in {
            "nothing": b"",
            "not UTF-8": good.replace(b"healthy", b"health\xff", 1),
            "a byte order mark": b"\xef\xbb\xbf" + good,
            "something after it": good + b"{}",
            "a field twice": good.replace(version, b'"status_version": 2, "status_version": 1,', 1),
            "NaN": good.replace(version, b'"status_version": NaN,', 1),
            "Infinity": good.replace(version, b'"status_version": Infinity,', 1),
            "a number of 5000 digits": good.replace(version, b'"status_version": ' + b"1" * 5000 + b",", 1),
            "nested too deep": b"[" * 100_000,
            "a page": b"<!doctype html><title>status</title>",
        }.items():
            with self.subTest(name):
                self.assertNotEqual(answer, good)
                self.assertTrue(refused(overview.document, answer))

    def test_a_tenants_summary_is_held_to_the_same(self):
        good = load(FIXTURES / "status" / "taken" / "a-server.json")
        tenant = next(one for one in good["instances"] if one["kind"] == "tenant")
        for name, summary in load(FIXTURES / "summary" / "refused.json").items():
            if summary is None:
                continue
            with self.subTest(name):
                tenant["summary"] = summary
                self.assertTrue(refused(overview.status, good))


SMALL = load(FIXTURES / "summary" / "taken" / "castle.json")
PLATFORM = [f"platform\t{service}\thealthy" for service in ("caddy", "keycloak", "keycloak-db", "dns")]
FACTS = {
    "now": "2026-11-22T19:00:00Z",
    "sign_in": "2027-01-30T04:00:00Z",
    "backup": "2026-11-22T17:05:00Z",
    "reboot": "false",
    "failed": "0",
    "disk": "41",
    "renewal": "true",
    "backups": "true",
}


def instance_line(codename, kind="tenant", health="healthy", image="95bf92f77c9e", ends="2027-02-20T17:36:45Z", summary=""):
    if not isinstance(summary, str):
        summary = json.dumps(summary, separators=(",", ":"))
    return "\t".join(("instance", codename, kind, health, image, ends, summary))


def fact_lines(**changed):
    """The facts of a server on which all is well, some of them otherwise or (None) not there."""
    return [f"fact\t{name}\t{value}" for name, value in (FACTS | changed).items() if value is not None]


def assembled(lines):
    """What status.jq makes of the lines `castlectl status` gathers: the document, or None where it ends the run."""
    done = subprocess.run(
        ["jq", "-cnR", "-L", str(COMPOSE), 'include "status"; status'],
        input="".join(line + "\n" for line in lines),
        capture_output=True,
        text=True,
    )
    return done.stdout.encode() if done.returncode == 0 else None


def left_out(lines):
    """How much of the lines status.jq says is not in the document it made of them."""
    done = subprocess.run(
        ["jq", "-cnR", "-L", str(COMPOSE), 'include "status"; assembled | .left_out'],
        input="".join(line + "\n" for line in lines),
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(done.stdout)


def largest(projects=500):
    """A summary of that many projects with every number as large as it may be."""
    one = {
        "drafts": 1_000_000,
        "published": 1_000_000,
        "by_severity": dict.fromkeys(overview.SEVERITIES, 1_000_000),
        "comments": dict.fromkeys(overview.ROLES, 1_000_000),
        "last_activity": "2026-11-22T17:37:02Z",
    }
    return {
        "summary_version": 1,
        "generated_at": "2026-11-22T18:59:00Z",
        "window_days": 7,
        "projects": [dict(one, id=10**12 - at) for at in range(projects)],
        "activity": [],
    }


class Assembling(unittest.TestCase):
    def taken(self, lines):
        """The document made of the lines, which the overview has to take as it is."""
        made = assembled(lines)
        self.assertIsNotNone(made, "status.jq ended the run")
        self.assertLessEqual(len(made), overview.ANSWER_BYTES)
        said = overview.document(made)
        self.assertEqual(plain(said), json.loads(made))
        return said

    def test_an_ordinary_server(self):
        said = self.taken(
            [
                instance_line("amber", summary=SMALL),
                instance_line("birch", "canary", image=""),
                instance_line("dune", "cert-only", "", "", ""),
                *PLATFORM,
                *fact_lines(),
            ]
        )
        self.assertEqual([one["kind"] for one in said["instances"]], ["tenant", "canary", "cert-only"])
        self.assertEqual(plain(said["instances"][0]["summary"]), by_jq(SMALL))
        self.assertEqual([one["service"] for one in said["platform"]], ["caddy", "keycloak", "keycloak-db", "dns"])
        self.assertEqual(
            said["host"],
            {"reboot_required": False, "failed_units": 0, "disk_used_percent": 41, "renewal_timer": True, "backup_timer": True},
        )
        self.assertEqual(plain(said["newest_backup"]), FACTS["backup"])

    def test_what_was_not_read_or_cannot_be_is_null(self):
        where = {
            "disk": lambda said: said["host"]["disk_used_percent"],
            "failed": lambda said: said["host"]["failed_units"],
            "reboot": lambda said: said["host"]["reboot_required"],
            "renewal": lambda said: said["host"]["renewal_timer"],
            "backups": lambda said: said["host"]["backup_timer"],
            "backup": lambda said: plain(said["newest_backup"]),
            "sign_in": lambda said: plain(said["sign_in_certificate_ends"]),
        }
        for name, value, wanted in (
            ("disk", "100", 100),
            ("disk", "101", None),
            ("disk", "", None),
            ("disk", None, None),
            ("disk", "-1", None),
            ("disk", "4.5", None),
            ("disk", "4e1", None),
            ("failed", "100000", 100_000),
            ("failed", "100001", None),
            ("failed", "999999999", None),
            ("failed", "9999999999", None),
            ("failed", "many", None),
            ("reboot", "true", True),
            ("reboot", "yes", None),
            ("reboot", "", None),
            ("renewal", "active", None),
            ("backups", "false", False),
            ("backups", None, None),
            ("backup", "", None),
            ("backup", "castle-backup-20261122-170500.tar.age", None),
            ("backup", "2026-02-30T00:00:00Z", None),
            ("sign_in", "tomorrow", None),
            ("sign_in", "2027-01-30T04:00:00+01:00", None),
        ):
            with self.subTest(f"{name}: {value!r}"):
                said = self.taken([instance_line("amber"), *PLATFORM, *fact_lines(**{name: value})])
                self.assertEqual(where[name](said), wanted)
        said = self.taken([instance_line("amber"), "platform\tcaddy\tup", *PLATFORM[1:3], *fact_lines()])
        self.assertEqual([one["health"] for one in said["platform"]], [None, "healthy", "healthy"])

    def test_an_instance_has_what_its_kind_has_and_nothing_that_is_not_one(self):
        said = self.taken(
            [
                instance_line("amber", health="fine", image="castle:main", ends="soon", summary="{}"),
                instance_line("birch", "canary", image="95bf92f77c9e", summary=SMALL),
                instance_line("cedar", "cert-only", summary=SMALL),
                *PLATFORM,
                *fact_lines(),
            ]
        )
        nothing = {"health": None, "image": None, "certificate_ends": None, "summary": None}
        self.assertEqual(
            plain(said["instances"]),
            [
                {"codename": "amber", "kind": "tenant"} | nothing,
                {"codename": "birch", "kind": "canary"} | nothing | {"health": "healthy", "certificate_ends": "2027-02-20T17:36:45Z"},
                {"codename": "cedar", "kind": "cert-only"} | nothing,
            ],
        )

    def test_an_image_is_twelve_characters_of_a_digest_and_not_one_more(self):
        for image, wanted in (
            ("95bf92f77c9e", "95bf92f77c9e"),
            ("95bf92f77c9", None),
            ("95bf92f77c9e0", None),
            ("9" * 64, None),
            ("95BF92F77C9E", None),
            ("95bf92f77c9e\r", None),
        ):
            with self.subTest(image):
                said = self.taken([instance_line("amber", image=image), *PLATFORM, *fact_lines()])
                self.assertEqual(said["instances"][0]["image"], wanted)

    def test_what_is_left_out_is_counted(self):
        whole = [instance_line("amber", summary=SMALL), instance_line("birch", "canary", image=""), *PLATFORM, *fact_lines()]
        self.assertEqual(left_out(whole), {"names": 0, "summaries": 0})
        self.assertEqual(left_out([*whole, instance_line("amber"), instance_line("Amber")]), {"names": 2, "summaries": 0})
        many = [instance_line(f"large-{at}", summary=largest()) for at in range(10)] + [*PLATFORM, *fact_lines()]
        gone = sum(one["summary"] is None for one in self.taken(many)["instances"])
        self.assertTrue(0 < gone < 10)
        self.assertEqual(left_out(many), {"names": 0, "summaries": gone})
        self.assertEqual(left_out([instance_line("amber", summary=largest(501)), *PLATFORM, *fact_lines()])["summaries"], 0)

    def test_a_pool_name_that_is_no_codename_is_left_out(self):
        odd = ["Amber", "a", "a" * 32, "9lives", "am.ber", "am ber", "<script>", "\u00e4mber"]
        said = self.taken(
            [*map(instance_line, odd), instance_line("amber"), instance_line("b" * 31, "canary", image=""), *PLATFORM, *fact_lines()]
        )
        self.assertEqual([one["codename"] for one in said["instances"]], ["amber", "b" * 31])

    def test_a_name_given_twice_is_there_once_and_what_comes_after_the_500th_is_not(self):
        names = [f"pool-{at:03}" for at in range(600)]
        said = self.taken(
            [instance_line("pool-000"), *(instance_line(name, "canary", image="") for name in names), *PLATFORM, *fact_lines()]
        )
        self.assertEqual([one["codename"] for one in said["instances"]], names[:500])
        self.assertEqual(said["instances"][0]["kind"], "tenant")

    def test_the_largest_summary_there_is_has_room(self):
        said = self.taken([instance_line("amber", summary=largest()), *PLATFORM, *fact_lines()])
        self.assertEqual(len(said["instances"][0]["summary"]["projects"]), 500)
        self.assertGreater(len(json.dumps(by_jq(largest()))), 100_000)

    def test_a_summary_of_more_projects_than_one_carries_is_none(self):
        said = self.taken(
            [instance_line("amber", summary=largest(501)), instance_line("birch", summary=SMALL), *PLATFORM, *fact_lines()]
        )
        self.assertEqual([one["summary"] is None for one in said["instances"]], [True, False])

    def test_past_a_megabyte_the_largest_summaries_go_first_and_no_more_than_have_to(self):
        lines = [instance_line("small", summary=SMALL)]
        lines += [instance_line(f"large-{at}", summary=largest(500 - at)) for at in range(10)]
        lines += [*PLATFORM, *fact_lines()]
        made = assembled(lines)
        self.assertLessEqual(len(made), 1_000_000)
        said = self.taken(lines)
        gone = [one["codename"] for one in said["instances"] if one["summary"] is None]
        self.assertEqual(gone, [f"large-{at}" for at in range(len(gone))])
        self.assertTrue(0 < len(gone) < 10)
        last = len(json.dumps(by_jq(largest(500 - (len(gone) - 1))), separators=(",", ":")))
        self.assertGreater(len(made) - len("null") + last, 1_000_000)

    def test_lines_that_are_not_those_of_a_status_end_the_run(self):
        good = [instance_line("amber", summary=SMALL), *PLATFORM, *fact_lines()]
        self.taken(good)
        self.taken(["", *good, ""])
        for name, lines in {
            "no time": [good[0], *PLATFORM, *fact_lines(now=None)],
            "a time that is none": [good[0], *PLATFORM, *fact_lines(now="just now")],
            "a service missing": [good[0], PLATFORM[0], PLATFORM[2], *fact_lines()],
            "a service twice": [*good, PLATFORM[0]],
            "a service there is not": [*good, "platform\tpostfix\thealthy"],
            "a line of another kind": [*good, "motd\tall is well"],
            "a fact a status has not": [*good, "fact\tuptime\t3"],
            "a fact twice": [*good, "fact\tdisk\t3"],
            "a field short": ["instance\tamber\ttenant", *PLATFORM, *fact_lines()],
            "a tab in a value": [instance_line("amber", health="heal\tthy"), *PLATFORM, *fact_lines()],
            "a line end in a value": [instance_line("amber", health="heal\nthy"), *PLATFORM, *fact_lines()],
            "a kind there is not": [instance_line("amber", "guest"), *PLATFORM, *fact_lines()],
        }.items():
            with self.subTest(name):
                self.assertIsNone(assembled(lines))

    def test_whatever_is_gathered_the_overview_takes_what_is_made_of_it(self):
        chance = random.Random(20261122)
        values = [
            "", "healthy", "starting", "unhealthy", "true", "false", "0", "7", "100", "101", "999999999",
            "2027-02-20T17:36:45Z", "2026-02-30T00:00:00Z", "95bf92f77c9e", "castle:main", "<b>", "null", "{}",
            json.dumps(SMALL), json.dumps(dict(SMALL, title="x")),
        ]
        for _ in range(150):
            lines = [
                instance_line(
                    chance.choice(["amber", "birch", "cedar", "Amber", "a"]),
                    chance.choice(["tenant", "canary", "cert-only"]),
                    *(chance.choice(values) for _ in range(4)),
                )
                for _ in range(chance.randrange(5))
            ]
            lines += [f"platform\t{service}\t{chance.choice(values)}" for service in ("caddy", "keycloak", "keycloak-db")]
            lines += [f"fact\t{name}\t{chance.choice(values)}" for name in FACTS if name != "now"]
            self.taken([*lines, "fact\tnow\t2026-11-22T19:00:00Z"])


# Stand-ins that `castlectl status` finds on its path in place of the engine and
# of what it asks the host. They answer from files under engine/, which a test
# arranges.
ENGINE = r"""#!/bin/bash
E="$(dirname "$0")/../engine"
[[ -e "$E/down" ]] && { echo "Error: the engine does not answer" >&2; exit 125; }
case "$1 $2" in
  "container exists") [[ -e "$E/containers/$3" ]] ;;
  "inspect --format")
    [[ -e "$E/containers/$4" ]] || exit 125
    case "$3" in
      *State.Status*) cat "$E/containers/$4" ;;
      *ImageName*)    echo "registry.example.test/castle@sha256:95bf92f77c9e$(printf '0%.0s' {1..52})" ;;
    esac ;;
  "ps -a")
    service="${8##*=}"
    [[ -e "$E/services/$service.once-empty" ]] && { rm "$E/services/$service.once-empty"; exit 0; }
    cat "$E/services/$service" 2>/dev/null; exit 0 ;;
  "exec castle-"*)
    name="${2#castle-}"
    [[ -e "$E/slow/$name" ]] && sleep "$(cat "$E/slow/$name")"
    cat "$E/summary/$name" 2>/dev/null; exit 0 ;;
  "info --format") dirname "$E" ;;
  *) exit 125 ;;
esac
"""
SYSTEMCTL = """#!/bin/sh
case "$1" in is-active) echo active ;; esac
"""
DF = """#!/bin/sh
printf 'Use%%\\n 41%%\\n'
"""
# `s_client` answers with the certificate kept for the name; the rest is the real openssl.
OPENSSL = r"""#!/bin/bash
if [[ "$1" == s_client ]]; then
  name=""; while [[ $# -gt 0 ]]; do [[ "$1" == -servername ]] && name="$2"; shift; done
  cat "$(dirname "$0")/../engine/tls/$name.pem" 2>/dev/null; exit
fi
exec {openssl} "$@"
"""
BASE, SIGN_IN = "tenants.example.test", "auth.example.test"


class Gathering(unittest.TestCase):
    """`castlectl status` itself, on a server that is made of files: what it prints is a status the overview takes."""

    @classmethod
    def setUpClass(cls):
        kept = tempfile.TemporaryDirectory()
        cls.addClassCleanup(kept.cleanup)
        cls.certificates = pathlib.Path(kept.name)
        for name in (f"amber.{BASE}", f"birch.{BASE}", f"cedar.{BASE}", SIGN_IN):
            subprocess.run(
                ["openssl", "req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes",
                 "-keyout", os.devnull, "-out", str(cls.certificates / f"{name}.pem"), "-days", "30",
                 "-subj", f"/CN={name}", "-addext", f"subjectAltName=DNS:{name}"],
                check=True,
                capture_output=True,
            )

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.here = pathlib.Path(directory.name)
        for name in ("castlectl", "status.jq", "summary.jq"):
            shutil.copy(COMPOSE / name, self.here / name)
        (self.here / ".env").write_text(
            f"BASE_DOMAIN={BASE}\nKC_HOST={SIGN_IN}\nKC_PUBLIC_URL=https://{SIGN_IN}\nCASTLE_IMAGE=registry.example.test/castle:main\n"
        )
        self.engine = self.here / "engine"
        for part in ("bin", "backups", "engine/containers", "engine/services", "engine/summary", "engine/slow", "engine/tls"):
            (self.here / part).mkdir(parents=True)
        for name, script in (("podman", ENGINE), ("systemctl", SYSTEMCTL), ("df", DF), ("openssl", OPENSSL.replace("{openssl}", shutil.which("openssl")))):
            (self.here / "bin" / name).write_text(script)
            (self.here / "bin" / name).chmod(0o755)
        shutil.copytree(self.certificates, self.engine / "tls", dirs_exist_ok=True)
        (self.here / "backups" / "castle-backup-20261122-170500.tar.age").touch()
        # A tenant, a canary, a name that has a certificate and runs nothing, and one that has nothing.
        self.pool("amber", "birch", "dune", "elm")
        self.tenant("amber")
        self.runs("oauth2-proxy-birch")
        (self.here / "caddy" / "certs" / f"dune.{BASE}").mkdir(parents=True)
        (self.here / "caddy" / "certs" / f"dune.{BASE}" / "fullchain.pem").touch()
        for service in ("caddy", "keycloak", "keycloak-db"):
            (self.engine / "services" / service).write_text(f"compose_{service}_1\n")
            self.runs(f"compose_{service}_1")

    def pool(self, *names, ends="\n"):
        (self.here / "codenames.txt").write_text("# the pool\n" + "".join(name + ends for name in names))

    def runs(self, container, state="running/healthy"):
        (self.engine / "containers" / container).write_text(state + "\n")

    def tenant(self, codename, summary=SMALL):
        for container in (f"castle-{codename}", f"postgres-{codename}", f"oauth2-proxy-{codename}"):
            self.runs(container)
        (self.engine / "summary" / codename).write_text(json.dumps(summary) + "\n")

    def status(self, **settings):
        """Runs `castlectl status`: its exit status, the status it printed (or None) and what it said beside."""
        done = subprocess.run(
            ["bash", str(self.here / "castlectl"), "status"],
            env={"PATH": f"{self.here / 'bin'}:{os.environ['PATH']}", "HOME": str(self.here)} | settings,
            capture_output=True,
            timeout=120,
        )
        said = overview.document(done.stdout) if done.stdout else None
        return done.returncode, said, done.stderr.decode()

    def test_an_ordinary_server(self):
        code, said, beside = self.status()
        self.assertEqual((code, beside), (0, ""))
        amber, birch, dune = said["instances"]
        self.assertEqual([one["codename"] for one in said["instances"]], ["amber", "birch", "dune"])
        self.assertEqual([one["kind"] for one in said["instances"]], ["tenant", "canary", "cert-only"])
        self.assertEqual([one["health"] for one in said["instances"]], ["healthy", "healthy", None])
        self.assertEqual(amber["image"], "95bf92f77c9e")
        self.assertEqual(plain(amber["summary"]), by_jq(SMALL))
        for one in (amber, birch):
            self.assertGreater(one["certificate_ends"], said["generated_at"])
        self.assertIsNone(dune["certificate_ends"])
        self.assertGreater(said["sign_in_certificate_ends"], said["generated_at"])
        self.assertEqual([(one["service"], one["health"]) for one in said["platform"]], [(name, "healthy") for name in ("caddy", "keycloak", "keycloak-db")])
        self.assertEqual(plain(said["newest_backup"])[:4], "20" + plain(said["generated_at"])[2:4])
        self.assertEqual((said["host"]["disk_used_percent"], said["host"]["failed_units"]), (41, 0))
        self.assertEqual((said["host"]["renewal_timer"], said["host"]["backup_timer"]), (True, True))

    def test_an_engine_that_does_not_answer_gives_no_status(self):
        (self.engine / "down").touch()
        code, said, beside = self.status()
        self.assertNotEqual(code, 0)
        self.assertIsNone(said)
        self.assertIn("did not say whether castle-amber is there", beside)

    def test_a_tenant_that_prints_something_else_has_no_summary_and_the_rest_is_as_usual(self):
        self.tenant("cedar")
        self.pool("amber", "birch", "cedar")
        usual = self.status()[1]
        for name, printed in {
            "a title": json.dumps(dict(SMALL, title="<script>alert(1)</script>")),
            "a count as text": json.dumps(SMALL).replace('"drafts": 1', '"drafts": "1"', 1),
            "two summaries": json.dumps(SMALL) + "\n" + json.dumps(SMALL),
            "not JSON": "castle-cli: error",
            "nothing": "",
        }.items():
            (self.engine / "summary" / "amber").write_text(printed + "\n")
            with self.subTest(name):
                code, said, beside = self.status()
                self.assertEqual((code, beside), (0, ""))
                self.assertIsNone(said["instances"][0]["summary"])
                self.assertEqual(plain(said["instances"][2]["summary"]), by_jq(SMALL))
                for one in (usual, said):
                    one["instances"][0]["summary"] = one["generated_at"] = one["instances"][2]["summary"]["generated_at"] = None
                self.assertEqual(said, usual)

    def test_a_service_that_is_not_listed_for_a_moment_is_asked_for_once_more(self):
        (self.engine / "services" / "caddy.once-empty").touch()
        code, said, _ = self.status()
        self.assertEqual((code, said["platform"][0]), (0, {"service": "caddy", "health": "healthy"}))
        (self.engine / "services" / "caddy").write_text("")
        self.assertEqual(self.status()[1]["platform"][0], {"service": "caddy", "health": "unhealthy"})

    def test_a_pool_that_cannot_be_read_gives_no_status(self):
        (self.here / "codenames.txt").unlink()
        code, said, beside = self.status()
        self.assertNotEqual(code, 0)
        self.assertIsNone(said)
        self.assertIn("cannot be read", beside)
        self.pool("amber", "birch", "dune", ends="\r\n")
        code, said, beside = self.status()
        self.assertNotEqual(code, 0)
        self.assertIsNone(said)
        self.assertIn("none of the 3 names", beside)

    def test_names_that_are_no_codenames_are_not_asked_about_and_it_says_how_many(self):
        self.pool("amber", "Amber", "-rf", "../amber", "am ber", "birch", "amber")
        code, said, beside = self.status()
        self.assertEqual(code, 0)
        self.assertEqual([one["codename"] for one in said["instances"]], ["amber", "birch"])
        self.assertIn("leaves out 4 pool names that are not of a codename's shape", beside)
        self.assertIn("leaves out 1 pool names: listed twice", beside)

    def test_summaries_left_out_for_size_are_counted_beside_the_status(self):
        names = [f"large-{at}" for at in range(10)]
        for name in names:
            self.tenant(name, largest())
        self.pool(*names)
        code, said, beside = self.status()
        gone = sum(one["summary"] is None for one in said["instances"])
        self.assertTrue(code == 0 and 0 < gone < 10)
        self.assertIn(f"leaves out {gone} summaries, the largest", beside)

    def test_past_its_time_a_run_asks_no_more_and_says_how_much_it_left(self):
        self.tenant("cedar")
        self.pool("amber", "birch", "cedar")
        (self.engine / "slow" / "amber").write_text("2\n")
        code, said, beside = self.status(STATUS_SECONDS="1")
        self.assertEqual(code, 0)
        amber, birch, cedar = said["instances"]
        self.assertEqual(plain(amber["summary"]), by_jq(SMALL))
        self.assertEqual((cedar["summary"], amber["certificate_ends"], birch["certificate_ends"], cedar["certificate_ends"]), (None,) * 4)
        self.assertIsNone(said["sign_in_certificate_ends"])
        self.assertEqual([one["health"] for one in said["instances"]], ["healthy"] * 3)
        self.assertIn("ran out of its 1 seconds: 5 summaries and certificates were not asked for", beside)
        for never in ("soon", "1e3", "a[$(touch " + str(self.here / "ran") + ")]"):
            self.assertNotEqual(self.status(STATUS_SECONDS=never)[0], 0)
        self.assertFalse((self.here / "ran").exists())


def stub(test, script):
    """Puts a shell script where the overview finds ssh, for one test."""
    directory = tempfile.TemporaryDirectory()
    test.addCleanup(directory.cleanup)
    path = pathlib.Path(directory.name, "ssh")
    path.write_text(script + "\n", encoding="utf-8")
    test.addCleanup(setattr, overview, "SSH", overview.SSH)
    overview.SSH = ["/bin/sh", str(path)]


# For a stub of ssh: the file it was told to write its own diagnostics to.
LOG = 'log=; take=; for a in "$@"; do [ -n "$take" ] && log=$a && take=; [ "$a" = -E ] && take=1; done; '

SERVER = {
    "name": "castle-1",
    "host": "203.0.113.10",
    "port": 2222,
    "user": "castle-status",
    "key": "/keys/status",
    "known_hosts": "/keys/known_hosts",
    "base_domain": "tenants.example.com",
    "clients": {"amber": "Example BV"},
}


class Asking(unittest.TestCase):
    def reason(self, script):
        stub(self, script)
        with self.assertRaises(overview.NoAnswer) as raised:
            overview.ask(SERVER)
        return str(raised.exception)

    def test_ssh_is_told_everything_and_the_server_nothing(self):
        stub(self, 'for a in "$@"; do printf "%s\\n" "$a"; done')
        said = overview.ask(SERVER).decode().splitlines()
        self.assertEqual(said[-2:], ["--", "203.0.113.10"])
        self.assertEqual(said[:2], ["-F", "none"])
        options = {said[at + 1] for at, word in enumerate(said) if word == "-o"}
        for wanted in (
            "BatchMode=yes",
            "StrictHostKeyChecking=yes",
            "UserKnownHostsFile=/keys/known_hosts",
            "GlobalKnownHostsFile=/dev/null",
            "UpdateHostKeys=no",
            "IdentitiesOnly=yes",
            "IdentityAgent=none",
            "IdentityFile=/keys/status",
            "ClearAllForwardings=yes",
            "LogLevel=FATAL",
        ):
            self.assertIn(wanted, options)
        self.assertEqual(said[said.index("-p") + 1], "2222")
        self.assertEqual(said[said.index("-l") + 1], "castle-status")

    def test_only_what_the_stub_is_given_reaches_it(self):
        os.environ["SSH_AUTH_SOCK"] = "/run/agent"
        self.addCleanup(os.environ.pop, "SSH_AUTH_SOCK")
        stub(self, "env")
        names = {line.split("=")[0] for line in overview.ask(SERVER).decode().splitlines()}
        self.assertLessEqual(names, {"PATH", "LC_ALL", "PWD", "OLDPWD", "SHLVL", "_"})

    def test_an_answer_that_is_too_large_is_not_read_to_the_end(self):
        started = time.monotonic()
        self.assertIn("too large", self.reason("exec head -c 50000000 /dev/zero"))
        self.assertIn("too large", self.reason("while :; do echo 0123456789abcdef0123456789abcdef; done"))
        self.assertLess(time.monotonic() - started, 10)

    def test_a_server_that_does_not_answer_is_not_waited_for(self):
        self.addCleanup(setattr, overview, "ANSWER_SECONDS", overview.ANSWER_SECONDS)
        overview.ANSWER_SECONDS = 1
        started = time.monotonic()
        self.assertIn("in time", self.reason("sleep 30"))
        self.assertIn("in time", self.reason("exec >&- 2>&-; sleep 30"))
        self.assertLess(time.monotonic() - started, 8)

    def test_the_reason_is_what_ssh_itself_said_in_this_programs_words(self):
        def ssh_says(words, status=255):
            return f"{LOG}printf '%s\\r\\n' '{words}' >> \"$log\"; exit {status}"

        def the_far_side_prints(words, status=255):
            return f"echo '{words}' >&2; echo '{words}'; exit {status}"

        denied = "castle-status@203.0.113.10: Permission denied (publickey)."
        for script, words in (
            (ssh_says("Host key verification failed."), "another host key than the one pinned here"),
            (ssh_says(denied), "does not take this key"),
            (ssh_says("ssh: connect to host 203.0.113.10 port 2222: Connection refused"), "no connection"),
            (ssh_says("Received disconnect from 203.0.113.10 port 2222:2: Host key verification failed."), "no connection"),
            (ssh_says("root@203.0.113.10: Permission denied (publickey)."), "no connection"),
            (ssh_says("castle-status@203.0.113.11: Permission denied (publickey)."), "no connection"),
            (ssh_says("x " + denied), "no connection"),
            (the_far_side_prints("Host key verification failed."), "no connection"),
            (the_far_side_prints(denied), "no connection"),
            (the_far_side_prints("cat: /var/lib/castle-status/status.json: No such file or directory", 1), "has none to give"),
            (the_far_side_prints("<script>alert(1)</script>"), "no connection"),
            (the_far_side_prints("<script>alert(1)</script>", 7), "has none to give"),
        ):
            with self.subTest(script):
                said = self.reason(script)
                self.assertIn(words, said)
                for never in ("script", "203.0.113.10", "nothing was asked"):
                    self.assertNotIn(never, said)

    def test_words_the_far_side_puts_into_what_ssh_says_do_not_choose_the_reason(self):
        def ssh_says(written):
            """A stub that writes this to ssh's own file, with printf's escapes for line ends."""
            return f"{LOG}printf '{written}' >> \"$log\"; exit 255"

        key = "Host key verification failed."
        denied = "castle-status@203.0.113.10: Permission denied (publickey)."
        for written, words in (
            (f"{key}\\r\\n", "another host key"),
            (f"{key}\\n", "another host key"),
            (f"{denied}\\r\\n", "does not take this key"),
            (f"x\\r\\n{key}\\r\\n", "no connection"),
            (f"{key}\\r\\n{key}\\r\\n", "no connection"),
            (f"{key}\\r\\nx\\r\\n", "no connection"),
            (f" {key}\\r\\n", "no connection"),
            (key, "no connection"),
            (f"Received disconnect from 203.0.113.10 port 2222:2: x\\r\\n{key}\\r\\n", "no connection"),
            (f"Received disconnect from 203.0.113.10 port 2222:2: x\\r\\n{denied}\\r\\n", "no connection"),
            ("castle-status@203.0.113.10: Permission denied (x).\\r\\n" + key + "\\r\\n(y).\\r\\n", "does not take this key"),
            ("", "no connection"),
        ):
            with self.subTest(written):
                self.assertIn(words, self.reason(ssh_says(written)))

    def test_a_host_written_with_capitals_is_the_host_ssh_names_in_small_letters(self):
        stub(self, f"{LOG}printf 'castle-status@castle-one.example.com: Permission denied (publickey).\\r\\n' >> \"$log\"; exit 255")
        with self.assertRaises(overview.NoAnswer) as raised:
            overview.ask(dict(SERVER, host="Castle-One.Example.COM"))
        self.assertIn("does not take this key", str(raised.exception))

    def test_what_ssh_says_cannot_grow_past_its_limit(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        size = pathlib.Path(directory.name, "size")
        started = time.monotonic()
        said = self.reason(f'{LOG}head -c 50000000 /dev/zero >> "$log"; wc -c < "$log" > {size}; exit 255')
        self.assertIn("no connection", said)
        self.assertEqual(int(size.read_text()), overview.DIAGNOSTIC_BYTES)
        self.assertLess(time.monotonic() - started, 10)

    def test_what_ssh_says_goes_to_a_file_of_its_own_that_does_not_stay(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.addCleanup(setattr, tempfile, "tempdir", tempfile.tempdir)
        tempfile.tempdir = directory.name
        stub(self, LOG + 'stat -c "%a %n" "$log"')
        mode, path = overview.ask(SERVER).decode().split()
        self.assertEqual((mode, os.path.dirname(path)), ("600", directory.name))
        stub(self, LOG + "exit 255")
        with self.assertRaises(overview.NoAnswer):
            overview.ask(SERVER)
        self.assertEqual([name for name in os.listdir(directory.name) if name.startswith("overview-ssh-")], [])

    def test_a_server_that_complains_at_length_is_still_heard(self):
        stub(self, "head -c 5000000 /dev/zero >&2; echo answered")
        self.assertEqual(overview.ask(SERVER), b"answered\n")


class Tags(html.parser.HTMLParser):
    def __init__(self, page):
        super().__init__(convert_charrefs=True)
        self.tags, self.text, self.inside = [], [], []
        self.feed(page)
        self.close()

    def handle_starttag(self, tag, attrs):
        self.tags.append((tag, dict(attrs)))
        self.inside.append(tag)

    def handle_endtag(self, tag):
        self.inside.pop()

    def handle_data(self, data):
        if "style" not in self.inside:
            self.text.append(data)

    def named(self, tag):
        return [attributes for name, attributes in self.tags if name == tag]


class Page(unittest.TestCase):
    def setUp(self):
        self.said = overview.document((FIXTURES / "status" / "taken" / "a-server.json").read_bytes())
        self.now = self.said["generated_at"] + datetime.timedelta(minutes=3)

    def page(self, *results, now=None):
        return overview.page(list(results), now or self.now)

    def test_it_is_one_page_without_script_that_asks_for_nothing(self):
        page = self.page((SERVER, self.said))
        tags = Tags(page)
        self.assertTrue(page.startswith("<!doctype html>\n<html lang=\"en\">"))
        for tag, attributes in tags.tags:
            self.assertIn(tag, {"html", "head", "meta", "title", "style", "body", "header", "main", "h2", "h3", "p", "div", "ul", "li", "a", "span", "code", "strong", "table", "thead", "tbody", "tr", "th", "td"})
            self.assertLessEqual(attributes.keys(), {"class", "href", "rel", "lang", "charset", "http-equiv", "content", "name", "title", "rowspan", "colspan"})
        policy = next(m["content"] for m in tags.named("meta") if m.get("http-equiv") == "Content-Security-Policy")
        style = page.split("<style>")[1].split("</style>")[0]
        digest = base64.b64encode(hashlib.sha256(style.encode()).digest()).decode()
        self.assertEqual(policy, f"default-src 'none'; style-src 'sha256-{digest}'; base-uri 'none'; form-action 'none'")
        self.assertEqual(page.count("<style>"), 1)

    def test_every_link_goes_to_a_codename_of_the_server_it_is_listed_under(self):
        tags = Tags(self.page((SERVER, self.said)))
        links = [a["href"] for a in tags.named("a")]
        self.assertIn("https://amber.tenants.example.com/", links)
        self.assertIn("https://amber.tenants.example.com/projects/1", links)
        self.assertIn("https://amber.tenants.example.com/findings/1", links)
        for link in links:
            self.assertRegex(link, r"\Ahttps://amber\.tenants\.example\.com/((projects|findings)/[0-9]+)?\Z")
        for a in tags.named("a"):
            self.assertEqual(a["rel"], "noreferrer")

    def test_what_is_new_is_on_it(self):
        text = " ".join(Tags(self.page((SERVER, self.said))).text)
        self.assertIn("Example BV", text)
        for wanted in ("Project 1", "1 high", "1 from the client", "1 from us", "The client commented on", "We commented on"):
            self.assertIn(wanted, text)
        self.assertIn("2 canaries, 1 with a certificate only", text)

    def test_what_is_new_is_of_every_server_newest_first_and_no_longer_than_it_says(self):
        def with_comments(minutes):
            said = copy.deepcopy(self.said)
            said["instances"][0]["summary"]["activity"] = [
                {"at": self.now - datetime.timedelta(minutes=each), "project_id": 1, "finding_id": each, "by": "client"}
                for each in minutes
            ]
            return said

        one, two = dict(SERVER, name="castle-1"), dict(SERVER, name="castle-2", clients={"amber": "Sample Ltd"})
        page = self.page((one, with_comments(range(10, 60, 2))), (two, with_comments(range(11, 61, 2))))
        listed = [a["href"].rsplit("/", 1)[1] for a in Tags(page).named("a") if "/findings/" in a["href"]]
        self.assertEqual(listed, [str(minute) for minute in range(10, 10 + overview.NEWS_SHOWN)])
        text = " ".join(Tags(page).text)
        self.assertIn("Sample Ltd", text)
        self.assertIn("10 min ago", text)

    def test_a_time_is_said_as_of_when_the_page_was_made(self):
        for apart, wanted in ((0, "just now"), (89, "just now"), (90, "1 min ago"), (7200, "2 h ago"), (36 * 3600, "1 days ago"), (-600, "10 min ahead")):
            made = overview.ago(self.now - datetime.timedelta(seconds=apart), self.now)
            self.assertIn(f">{wanted}<", made)
            self.assertIn(f'title="{overview.when(self.now - datetime.timedelta(seconds=apart))}"', made)

    def test_names_from_the_configuration_are_text(self):
        server = dict(SERVER, name="<b>one</b>", clients={"amber": '<img src=x onerror="alert(1)">'})
        page = self.page((server, self.said))
        self.assertNotIn("<img", page)
        self.assertNotIn("<b>", page)
        self.assertIn("&lt;img src=x onerror=&quot;alert(1)&quot;&gt;", page)
        self.assertEqual(Tags(page).named("img"), [])

    def test_a_status_that_is_old_is_called_old(self):
        fresh = self.page((SERVER, self.said), now=self.said["generated_at"] + datetime.timedelta(minutes=15))
        self.assertNotIn("Old:", fresh)
        self.assertNotIn("stale", fresh.split("</style>")[1])
        old = self.page((SERVER, self.said), now=self.said["generated_at"] + datetime.timedelta(minutes=16))
        self.assertIn("Old: the newest status this server has is of", old)
        self.assertIn('<div class="server stale">', old)
        self.assertIn('<tr class="stale">', old)
        ahead = self.page((SERVER, self.said), now=self.said["generated_at"] - datetime.timedelta(minutes=16))
        self.assertIn("A clock is wrong: this status is of", ahead)
        self.assertIn('<div class="server stale">', ahead)

    def test_a_server_without_a_status_shows_as_that_and_the_others_as_they_are(self):
        marker = "<script>alert('from the server')</script>"
        try:
            overview.document(marker.encode())
        except overview.Unrecognised as outcome:
            unrecognised = outcome
        page = self.page(
            (dict(SERVER, name="castle-1"), overview.NoAnswer("no connection")),
            (dict(SERVER, name="castle-2"), unrecognised),
            (dict(SERVER, name="castle-3"), self.said),
        )
        text = " ".join(Tags(page).text)
        self.assertIn("castle-1: No status: no connection.", text)
        self.assertIn("Not known: castle-1 gave no status.", text)
        self.assertIn("something that is not a status", text)
        self.assertNotIn("from the server", page)
        self.assertIn("Example BV", text)
        self.assertIn("3 server(s), 3 to look at", text)

    def test_what_needs_a_look_is_said(self):
        said = overview.document((FIXTURES / "status" / "taken" / "a-server-with-trouble.json").read_bytes())
        text = " ".join(Tags(self.page((SERVER, said), now=said["generated_at"])).text)
        for wanted in (
            "caddy is unhealthy",
            "amber is unhealthy",
            "amber gave no summary",
            "the certificate of amber could not be read",
            "the certificate of the sign-in host could not be read",
            "no backup was found",
            "a restart is waiting",
            "the certificate renewal timer is not active",
            "the backup timer is not active",
        ):
            self.assertIn(wanted, text)

    def test_a_server_that_leaves_things_out_is_noticed(self):
        said = copy.deepcopy(self.said)
        said["instances"] = [one for one in said["instances"] if one["kind"] != "tenant"]
        text = " ".join(Tags(self.page((SERVER, said))).text)
        self.assertIn("amber has a name here, and the server lists no such tenant", text)

    def test_a_tenant_without_a_name_an_image_or_a_summary_of_now_is_said(self):
        said = copy.deepcopy(self.said)
        tenant = next(one for one in said["instances"] if one["kind"] == "tenant")
        tenant["summary"]["generated_at"] = said["generated_at"] - datetime.timedelta(minutes=15)
        text = " ".join(Tags(self.page((SERVER, said))).text)
        for unwanted in ("has no name here", "runs could not be read", "the summary of amber"):
            self.assertNotIn(unwanted, text)
        tenant["image"] = None
        tenant["summary"]["generated_at"] = said["generated_at"] - datetime.timedelta(minutes=16)
        text = " ".join(Tags(self.page((dict(SERVER, clients={}), said))).text)
        for wanted in (
            "amber is a tenant on the server and has no name here",
            "what amber runs could not be read",
            "the summary of amber is of",
        ):
            self.assertIn(wanted, text)

    def test_a_good_server_needs_no_look(self):
        said = copy.deepcopy(self.said)
        said["host"] = {"reboot_required": False, "failed_units": 0, "disk_used_percent": 40, "renewal_timer": True, "backup_timer": True}
        said["sign_in_certificate_ends"] = self.now + datetime.timedelta(days=60)
        text = " ".join(Tags(self.page((SERVER, said))).text)
        self.assertIn("Nothing needs a look.", text)
        self.assertIn("1 server(s), nothing to look at", text)
        self.assertNotIn("Needs a look", text)

    def test_the_builder_escapes_text_and_attributes(self):
        made = overview.el("a", "<b>&</b>", overview.el("code", "x"), href='"><script>alert(1)</script>', class_=None)
        self.assertEqual(
            made,
            '<a href="&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;">&lt;b&gt;&amp;&lt;/b&gt;<code>x</code></a>',
        )
        self.assertNotIsInstance(made + made, overview.Markup)
        self.assertEqual(overview.el("p", made + ""), "<p>" + html.escape(made) + "</p>")


class FromAskingToThePage(unittest.TestCase):
    def test_a_fixture_through_a_stub(self):
        stub(self, f"cat {FIXTURES / 'status' / 'taken' / 'a-server.json'}")
        said = overview.collect(SERVER)
        self.assertEqual(said["instances"][0]["codename"], "amber")
        stub(self, "echo '{\"status_version\": 1}'")
        self.assertIsInstance(overview.collect(SERVER), overview.Unrecognised)
        stub(self, "exit 255")
        self.assertIsInstance(overview.collect(SERVER), overview.NoAnswer)

    def test_the_page_is_put_in_place_in_one_step(self):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "index.html")
            overview.write(path, "one")
            overview.write(path, "two")
            self.assertEqual(pathlib.Path(path).read_text(encoding="utf-8"), "two")
            self.assertEqual(os.listdir(directory), ["index.html"])
            self.assertEqual(os.stat(path).st_mode & 0o777, 0o600)


class Configuration(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.directory = directory.name
        for name in ("key", "known_hosts"):
            pathlib.Path(self.directory, name).touch(mode=0o600)
        self.example = (HERE / "overview.example.toml").read_text(encoding="utf-8")

    def configured(self, text):
        path = os.path.join(self.directory, "overview.toml")
        with open(path, "w", encoding="utf-8") as out:
            for there, here in (
                ("/home/overview/.ssh/castle-status", f"{self.directory}/key"),
                ("/home/overview/.config/castle-overview/known_hosts", f"{self.directory}/known_hosts"),
                ("/home/overview/overview/index.html", f"{self.directory}/index.html"),
            ):
                text = text.replace(there, here)
            out.write(text)
        os.chmod(path, 0o600)
        return overview.configuration(path)

    def test_the_example_is_a_configuration(self):
        configured = self.configured(self.example)
        self.assertEqual(configured["output"], f"{self.directory}/index.html")
        self.assertEqual(configured["servers"][0]["clients"], {"amber": "Example BV"})
        self.assertEqual(configured["servers"][0]["port"], 22)
        self.assertEqual(tomllib.loads(self.example)["servers"][0].keys(), configured["servers"][0].keys())

    def test_what_does_not_fit_is_refused(self):
        for name, (old, new) in {
            "a host that reads as an option": ('host = "203.0.113.10"', 'host = "-oProxyCommand=id"'),
            "a host with a space": ('host = "203.0.113.10"', 'host = "203.0.113.10 -v"'),
            "a user with a space": ('user = "castle-status"', 'user = "castle-status -v"'),
            "a base domain with a path": ('base_domain = "tenants.example.com"', 'base_domain = "tenants.example.com/x"'),
            "a base domain with a scheme": ('base_domain = "tenants.example.com"', 'base_domain = "javascript:alert(1)"'),
            "a port of text": ("port = 22", 'port = "22"'),
            "a port out of range": ("port = 22", "port = 70000"),
            "a key that is not there": ("/home/overview/.ssh/castle-status", "/nonexistent/key"),
            "a setting it does not know": ('user = "castle-status"', 'user = "castle-status"\nproxy = "x"'),
            "a codename that is none": ('amber = "Example BV"', '"Amber Inc" = "Example BV"'),
            "a name over several lines": ('amber = "Example BV"', 'amber = "Example\\nBV"'),
            "no output": ('output = "/home/overview/overview/index.html"', ""),
            "a page in a directory that is not there": ("/home/overview/overview/index.html", "/nonexistent/index.html"),
            "a page beside the configuration": ("/home/overview/overview/index.html", "index.html"),
            "a page that is a directory": ('output = "/home/overview/overview/index.html"', f'output = "{self.directory}"'),
            "not TOML": ("[[servers]]", "[[servers"),
        }.items():
            with self.subTest(name):
                self.assertIn(old, self.example)
                with self.assertRaises(overview.Misconfigured):
                    self.configured(self.example.replace(old, new))

    def test_a_key_that_others_can_read_is_refused_before_ssh_refuses_it(self):
        self.configured(self.example)
        os.chmod(os.path.join(self.directory, "key"), 0o640)
        with self.assertRaises(overview.Misconfigured):
            self.configured(self.example)

    def test_servers_that_are_not_tables_are_refused_in_words(self):
        for servers in ('["castle-1"]', "[1, 2]", '"castle-1"', "[]", "[[]]"):
            with self.subTest(servers), self.assertRaises(overview.Misconfigured):
                self.configured(f'output = "{self.directory}/index.html"\nservers = {servers}\n')

    def test_a_configuration_that_others_can_read_is_refused(self):
        self.configured(self.example)
        path = os.path.join(self.directory, "overview.toml")
        for mode in (0o640, 0o604, 0o644):
            os.chmod(path, mode)
            with self.subTest(oct(mode)), self.assertRaises(overview.Misconfigured):
                overview.configuration(path)

    def test_a_path_that_ssh_would_read_as_more_than_a_path_is_refused(self):
        for name in ("key+of@castle-1.x", "key file", "key%h", "key#1", "key${HOME}", "k\u00e9y", "key'"):
            pathlib.Path(self.directory, name).touch(mode=0o600)
            text = self.example.replace("/home/overview/.ssh/castle-status", f"{self.directory}/{name}")
            with self.subTest(name):
                if name == "key+of@castle-1.x":
                    self.assertEqual(self.configured(text)["servers"][0]["key"], f"{self.directory}/{name}")
                else:
                    with self.assertRaises(overview.Misconfigured):
                        self.configured(text)

    def test_what_cannot_be_read_as_a_configuration_is_refused_in_words(self):
        path = pathlib.Path(self.directory, "overview.toml")
        for name, written in {
            "not UTF-8": b'output = "/tmp/\xff"\n',
            "arrays within arrays": b"servers = " + b"[" * 5000 + b"]" * 5000 + b"\n",
            "tables within tables": b"servers = " + b"{a = " * 5000 + b"1" + b"}" * 5000 + b"\n",
            "nothing": b"",
        }.items():
            path.write_bytes(written)
            path.chmod(0o600)
            with self.subTest(name), self.assertRaises(overview.Misconfigured):
                overview.configuration(str(path))

    def test_a_page_that_could_not_be_written_is_refused_before_any_server_is_asked(self):
        self.configured(self.example)
        nul = self.example.replace("/home/overview/overview/index.html", f"{self.directory}/index\\u0000.html")
        self.assertIn("\0", tomllib.loads(nul)["output"])
        with self.assertRaises(overview.Misconfigured):
            self.configured(nul)
        with unittest.mock.patch("os.access", return_value=False), self.assertRaises(overview.Misconfigured) as raised:
            self.configured(self.example)
        self.assertIn("cannot be written to", str(raised.exception))
        if os.geteuid() != 0:
            os.chmod(self.directory, 0o500)
            self.addCleanup(os.chmod, self.directory, 0o700)
            with self.assertRaises(overview.Misconfigured):
                overview.configuration(os.path.join(self.directory, "overview.toml"))

    def test_without_ssh_no_server_is_asked_and_it_says_so(self):
        self.configured(self.example)
        self.addCleanup(setattr, overview, "SSH", overview.SSH)
        overview.SSH = ["castle-overview-has-no-such-ssh"]
        asked = unittest.mock.Mock()
        argv = ["overview", "--config", os.path.join(self.directory, "overview.toml")]
        with unittest.mock.patch.object(sys, "argv", argv), unittest.mock.patch.object(overview, "collect", asked):
            with self.assertRaises(SystemExit) as raised:
                overview.main()
        self.assertIn("castle-overview-has-no-such-ssh", str(raised.exception))
        asked.assert_not_called()
        self.assertFalse(os.path.exists(os.path.join(self.directory, "index.html")))

    def test_two_servers_need_two_names(self):
        twice = self.example + "\n" + self.example.split("# One table for every castle server.")[1]
        with self.assertRaises(overview.Misconfigured):
            self.configured(twice)
        self.assertEqual(len(self.configured(twice.replace('name = "castle-1"', 'name = "castle-2"', 1))["servers"]), 2)


if __name__ == "__main__":
    unittest.main()
