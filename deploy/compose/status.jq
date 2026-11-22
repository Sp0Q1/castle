# How a server is (`castlectl status`), put together from the lines castlectl
# gathers: one for each instance, one for each service of the platform and one
# for each fact about the host, their fields separated by tabs.
#
# Every value is held to what a status has there, as the overview reads it
# (../overview/overview.py). One that is not becomes null. A pool name that is
# not of a codename's shape is left out, as is one given twice and what comes
# after the 500th. A line that is none of these three ends the run. And the
# document stays within what the overview reads of an answer: where it would
# not, the largest summaries go first. `assembled` gives the document and how
# much of what was gathered is not in it, for castlectl to say so.

include "summary";

def moment: first(instant) // null;
def health: if IN("healthy", "starting", "unhealthy") then . else null end;
def flag: if . == "true" then true elif . == "false" then false else null end;
def upto($most):
  if test("\\A[0-9]{1,9}\\z") then tonumber | if . <= $most then . else null end else null end;

def instance:
  . as [$line, $codename, $kind, $health, $image, $ends, $summary]
  | if $kind | IN("tenant", "canary", "cert-only") then . else error("an instance of no kind") end
  | {codename: $codename, kind: $kind,
     health: (if $kind == "cert-only" then null else $health | health end),
     image: (if $kind == "tenant" and ($image | test("\\A[0-9a-f]{12}\\z")) then $image else null end),
     certificate_ends: (if $kind == "cert-only" then null else $ends | moment end),
     summary: (if $kind == "tenant" then $summary | (try fromjson catch null) | first(summary) // null else null end)};

# The document without the summaries that would take it past $most bytes.
def fitting($most):
  (.instances | map(if .summary == null then 0 else .summary | tojson | utf8bytelength end)) as $sizes
  | ((.instances |= map(.summary = null)) | tojson | utf8bytelength) as $bare
  | (reduce ($sizes | to_entries | map(select(.value > 0)) | sort_by(-.value))[] as $one
       ({over: ($bare + ($sizes | map(select(. > 0) - 4) | add // 0) - $most), gone: []};
        if .over > 0 then {over: (.over - $one.value + 4), gone: (.gone + [$one.key])} else . end)
     | .gone) as $gone
  | reduce $gone[] as $at (.; .instances[$at].summary = null);

def assembled:
  [inputs | select(length > 0) | split("\t")] as $lines
  | if all($lines[]; (.[0] == "instance" and length == 7) or (.[0] | IN("platform", "fact")) and length == 3)
    then . else error("a line that is none of a status's") end
  | (reduce ($lines[] | select(.[0] == "fact")) as [$line, $name, $value] ({};
       if has($name) or ($name | IN("now", "sign_in", "backup", "reboot", "failed", "disk", "renewal", "backups") | not)
       then error("a fact that a status has not, or has once") else .[$name] = $value end)) as $fact
  | [$lines[] | select(.[0] == "platform") | {service: .[1], health: (.[2] | health)}] as $platform
  | ($platform | map(.service)) as $services
  | if ($services | unique | length) == ($services | length)
       and ($services - ["caddy", "keycloak", "keycloak-db", "dns"]) == []
       and (["caddy", "keycloak", "keycloak-db"] - $services) == []
    then . else error("the services are not those of a platform") end
  | (reduce ($lines[] | select(.[0] == "instance")) as $line ({seen: {}, list: []};
       if ($line[1] | test("\\A[a-z][a-z0-9-]{1,30}\\z") | not) or .seen[$line[1]] or (.list | length) >= 500 then .
       else .seen[$line[1]] = true | .list += [$line | instance] end)
     | .list) as $instances
  | {status_version: 1,
     generated_at: ($fact.now // "" | first(instant) // error("no time for the status")),
     platform: $platform,
     sign_in_certificate_ends: ($fact.sign_in // "" | moment),
     instances: $instances,
     newest_backup: ($fact.backup // "" | moment),
     host: {reboot_required: ($fact.reboot // "" | flag),
            failed_units: ($fact.failed // "" | upto(100000)),
            disk_used_percent: ($fact.disk // "" | upto(100)),
            renewal_timer: ($fact.renewal // "" | flag),
            backup_timer: ($fact.backups // "" | flag)}}
  | fitting(1000000) as $document
  | def summaries: [.instances[] | select(.summary != null)] | length;
    {document: $document,
     left_out: {names: (([$lines[] | select(.[0] == "instance")] | length) - ($instances | length)),
                summaries: (({instances: $instances} | summaries) - ($document | summaries))}};

def status: assembled | .document;
