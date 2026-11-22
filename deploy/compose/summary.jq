# What a castle says about itself (its `summary` task), as its host takes it:
# rebuilt from the fields a summary has, each one checked. One that has
# anything else in it, or is not what it should be, gives nothing.

def whole($low; $high):
  select(type == "number" and . == floor and $low <= . and . <= $high)
  | if . == 0 then 0 else floor end;
def count: whole(0; 1000000);
def id: whole(1; 1000000000000);

def instant:
  select(type == "string" and test("\\A20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z\\z"))
  | select((try (strptime("%Y-%m-%dT%H:%M:%SZ") | mktime | strftime("%Y-%m-%dT%H:%M:%SZ")) catch null) == .);

def fields($names): select(type == "object" and (keys_unsorted | sort) == ($names | sort));
def list($most; one): select(type == "array" and length <= $most) | length as $n | map(one) | select(length == $n);

def by_severity:
  fields(["low", "medium", "elevated", "high", "extreme"])
  | {low: (.low | count), medium: (.medium | count), elevated: (.elevated | count),
     high: (.high | count), extreme: (.extreme | count)};

def by_role:
  fields(["manager", "staff", "client"])
  | {manager: (.manager | count), staff: (.staff | count), client: (.client | count)};

def project:
  fields(["id", "drafts", "published", "by_severity", "comments", "last_activity"])
  | {id: (.id | id), drafts: (.drafts | count), published: (.published | count),
     by_severity: (.by_severity | by_severity), comments: (.comments | by_role),
     last_activity: (.last_activity | if . == null then null else instant end)};

def activity($projects):
  fields(["at", "project_id", "finding_id", "by"])
  | {at: (.at | instant), project_id: (.project_id | id | select(IN($projects[]))),
     finding_id: (.finding_id | id), by: (.by | select(IN("manager", "staff", "client")))};

def summary:
  fields(["summary_version", "generated_at", "window_days", "projects", "activity"])
  | select(.summary_version == 1)
  | (.projects | list(500; project)) as $projects
  | ($projects | map(.id)) as $ids
  | select(($ids | unique | length) == ($ids | length))
  | {summary_version: 1, generated_at: (.generated_at | instant), window_days: (.window_days | whole(1; 90)),
     projects: $projects, activity: (.activity | list(50; activity($ids)))};
