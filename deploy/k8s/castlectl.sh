#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# castlectl — minimal per-client instance provisioner (the seed of the control
# plane). Wraps helm + kubectl so managing tenants is one command instead of a
# hand-run helm install + label dance.
#
#   castlectl allocate [--decoy]               # allocate a codename + provision it
#   castlectl provision <codename> [--decoy]   # provision a specific codename
#   castlectl pool                             # codename pool: free vs allocated
#   castlectl list                             # show all managed instances
#   castlectl deprovision <codename>           # remove one
#   castlectl deprovision --all                # remove every managed instance
#
# Config via env (local defaults shown):
#   IMG_REPO=castle  IMG_TAG=local  BASE_DOMAIN=127.0.0.1.nip.io
#   EXTERNAL_PORT   port in browser-facing URLs (default none = 443)
#
#   MODE selects the whole shape of a tenant:
#     local  built-in login (jwt) + sqlite + inline secrets. No IdP. (default)
#     proxy  oauth2-proxy + a per-tenant Keycloak realm + inline secrets +
#            sqlite. This is the SSO path, testable in the kind lab against a
#            running Keycloak. Needs KC_* below.
#     prod   like proxy, but secrets are delivered as a committed SealedSecret
#            (never --set) and the database is an internal per-tenant Postgres.
#            Additionally needs kubeseal on PATH and a SealedSecrets controller.
#
#   proxy/prod also need:
#     KC_PUBLIC_URL   browser-facing Keycloak base, e.g.
#                     https://keycloak.127.0.0.1.nip.io:8443
#     KC_INTERNAL     in-cluster Keycloak host:port the pod reaches directly,
#                     e.g. keycloak.castle-system.svc.cluster.local:8080
#     KC_ADMIN_USER / KC_ADMIN_PASS   admin creds for realm creation
#     KC_INSECURE=1   accept a self-signed ingress cert (lab only)
#
# PROVISIONING IS ONE COMMAND. That is the point: every step below is one a human
# would otherwise do by hand, and each has a way to go quietly wrong — a codename
# picked because it sounded nice (and so leaked something about the client), a
# realm built in the console with `sslRequired` left off, a client secret pasted
# into two places that then disagree, a helm install missing `secrets.external`
# so plaintext lands in the release.
#
# THE CLUSTER NEVER LEARNS WHICH CLIENT A CODENAME BELONGS TO. That mapping is
# what turns a public hostname back into "who", and it is the single most
# sensitive fact in the whole design — so it is kept OUT of here entirely. castle
# operates purely on opaque codenames; the app and the manifests never reference
# a real client name (grep them). You, the operator, record "codename -> client"
# wherever you already keep client records — offline, in a vault, or in the
# mTLS-isolated management plane once it exists. Storing it in the tenant cluster
# would mean one etcd read or leaked kubeconfig de-anonymises the entire
# portfolio, defeating the reason codenames exist. `allocate` prints the codename
# it picked; that is the only handle you need, and the only one that ever leaves.
# ---------------------------------------------------------------------------
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CHART="$HERE/tenant"
POOL="$HERE/codenames.txt"
IMG_REPO="${IMG_REPO:-castle}"
IMG_TAG="${IMG_TAG:-local}"
BASE_DOMAIN="${BASE_DOMAIN:-127.0.0.1.nip.io}"
MODE="${MODE:-local}"
EXTERNAL_PORT="${EXTERNAL_PORT:-}"
# Namespace the shared platform (Keycloak) lives in — used only as the egress
# target for proxy-mode tenants reaching the IdP. It holds no client identities.
CONTROL_NS="${CONTROL_NS:-castle-system}"
KC_REALM_PREFIX="${KC_REALM_PREFIX:-castle-}"
REALM_SH="$HERE/keycloak-realm.sh"
SEAL_SH="$HERE/secrets/seal-tenant.sh"
LABEL="castle.io/managed=true"

case "$MODE" in local|proxy|prod) ;; *) echo "castlectl: MODE must be local|proxy|prod (got '$MODE')" >&2; exit 1 ;; esac

die(){ echo "castlectl: $*" >&2; exit 1; }
ns_of(){ [[ "$2" == decoy ]] && echo "decoy-$1" || echo "tenant-$1"; }
need(){ command -v "$1" >/dev/null 2>&1 || die "$1 is required but not on PATH"; }
host_of(){ echo "$1.$BASE_DOMAIN${EXTERNAL_PORT:+:$EXTERNAL_PORT}"; }
rand_secret(){ openssl rand -base64 "${1:-32}" | tr -d '\n'; }

# Codenames currently in use, from the cluster itself rather than a local file:
# the cluster is the only thing that knows what really exists, and a stale local
# list would hand the same name to two clients.
allocated_codenames(){
  kubectl get ns -l "$LABEL" \
    -o jsonpath='{range .items[*]}{.metadata.labels.castle\.io/codename}{"\n"}{end}' 2>/dev/null \
  | grep -v '^$' | sort -u
}

pool_codenames(){ grep -vE '^\s*(#|$)' "$POOL"; }

# Picks a free codename AT RANDOM, not the next in the list. Sequential
# allocation would make position in the pool a timeline of onboarding order, and
# would let anyone who guesses the pool tell early clients from late ones — the
# same metadata leak the codenames exist to prevent.
next_codename(){
  local free
  free="$(comm -23 <(pool_codenames | sort) <(allocated_codenames))"
  [[ -n "$free" ]] || die "codename pool exhausted — add names to $(basename "$POOL") and issue their certificates in bulk"
  echo "$free" | shuf -n 1
}

# Split-horizon oauth2-proxy args: the browser is sent to Keycloak through the
# public ingress URL, while the pod redeems the code and fetches keys over the
# in-cluster address. --skip-oidc-discovery is what lets those two differ; auto
# discovery would force one URL to serve both and break one side.
oauth2_proxy_args(){
  local realm="$1"
  local kc_int="$KC_INTERNAL/realms/$realm/protocol/openid-connect"
  printf '[%s]' \
    "\"--skip-oidc-discovery=true\",\"--login-url=$KC_PUBLIC_URL/realms/$realm/protocol/openid-connect/auth\",\"--redeem-url=http://$kc_int/token\",\"--oidc-jwks-url=http://$kc_int/certs\",\"--profile-url=http://$kc_int/userinfo\""
}

# The chart's egress default-deny does not know about the in-cluster IdP (it only
# opens external :443). A proxy-mode pod must reach Keycloak to redeem codes, so
# this opens exactly that one hop and nothing else.
apply_idp_egress(){
  local ns="$1" app="castle-$2"
  kubectl apply -n "$ns" -f - >/dev/null <<NP
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: allow-idp-egress }
spec:
  podSelector: { matchLabels: { app: $app } }
  policyTypes: [Egress]
  egress:
    - to: [ { namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: $CONTROL_NS } } } ]
      ports: [ { protocol: TCP, port: 8080 } ]
NP
}

provision(){
  local codename="$1" role="${2:-tenant}"
  [[ -n "$codename" ]] || die "provision needs a <codename>"
  [[ "$codename" =~ ^[a-z][a-z0-9-]{1,30}$ ]] || die "invalid codename '$codename'"
  need helm; need kubectl; need openssl
  local ns; ns="$(ns_of "$codename" "$role")"
  local rel="castle-$codename" host; host="$(host_of "$codename")"
  echo "provisioning $role '$codename' [$MODE] -> https://$host (ns $ns)"

  local args=(
    --namespace "$ns" --create-namespace
    --set codename="$codename" --set baseDomain="$BASE_DOMAIN"
    ${EXTERNAL_PORT:+--set externalPort="$EXTERNAL_PORT"}
    --set image.repository="$IMG_REPO" --set image.tag="$IMG_TAG" --set image.pullPolicy=IfNotPresent
    --set runtimeClassName="${RUNTIME_CLASS:-}"
  )

  if [[ "$MODE" == local ]]; then
    # Built-in login (no sidecar => jwt), sqlite, inline secret. Lab only.
    args+=(--set oauth2Proxy.enabled=false
           --set database.internal=false
           --set database.externalUrl='sqlite:///app/uploads/castle.sqlite?mode=rwc'
           --set keycloak.issuerUrl=http://unused/realms/x
           --set secrets.jwtSecret="$(rand_secret 48)")
  else
    # proxy/prod: oauth2-proxy in front of a per-tenant Keycloak realm.
    [[ -n "${KC_PUBLIC_URL:-}" && -n "${KC_INTERNAL:-}" ]] || die "$MODE mode needs KC_PUBLIC_URL and KC_INTERNAL"
    local realm="${KC_REALM_PREFIX}${codename}"
    # ONE client secret, generated here, shared by the realm's client and the
    # tenant's oauth2-proxy. Generating it in two places is how they end up
    # disagreeing; this is the single source.
    local client_secret; client_secret="$(rand_secret 32)"
    echo "  creating realm $realm"
    CLIENT_SECRET="$client_secret" BASE_DOMAIN="$BASE_DOMAIN" EXTERNAL_PORT="$EXTERNAL_PORT" \
      KC_URL="$KC_PUBLIC_URL" KC_REALM_PREFIX="$KC_REALM_PREFIX" \
      "$REALM_SH" create "$codename" || die "realm creation failed for $codename"

    args+=(--set oauth2Proxy.enabled=true
           --set keycloak.issuerUrl="$KC_PUBLIC_URL/realms/$realm"
           --set keycloak.whitelistDomain="${KC_PUBLIC_URL#*://}"
           --set-json "oauth2Proxy.extraArgs=$(oauth2_proxy_args "$realm")")

    local jwt_secret cookie_secret db_password
    jwt_secret="$(rand_secret 48)"; cookie_secret="$(openssl rand -hex 16)"

    if [[ "$MODE" == proxy ]]; then
      # Lab SSO: inline secrets, sqlite.
      args+=(--set database.internal=false
             --set database.externalUrl='sqlite:///app/uploads/castle.sqlite?mode=rwc'
             --set secrets.jwtSecret="$jwt_secret"
             --set secrets.oauth2ClientSecret="$client_secret"
             --set secrets.oauth2CookieSecret="$cookie_secret")
    else
      # prod: internal Postgres + a committed SealedSecret, never --set.
      need kubeseal
      db_password="$(rand_secret 24)"
      kubectl get ns "$ns" >/dev/null 2>&1 || kubectl create ns "$ns" >/dev/null
      echo "  sealing secrets for $rel"
      DATABASE_URL="postgres://castle:$db_password@$rel-postgres:5432/castle" \
      JWT_SECRET="$jwt_secret" OAUTH2_CLIENT_SECRET="$client_secret" \
      OAUTH2_COOKIE_SECRET="$cookie_secret" DB_PASSWORD="$db_password" \
        "$SEAL_SH" "$rel" "$ns" --proxy --internal-db | kubectl apply -f - >/dev/null \
        || die "sealing/applying secrets failed for $rel"
      args+=(--set secrets.external=true
             --set database.internal=true)
    fi
  fi

  # Decoy behaviour is mode-dependent:
  #   local  -> honeypot=true: every local tenant is a jwt login form, so a
  #             capturing honeypot is indistinguishable from a real one AND logs
  #             the credentials tried against it.
  #   proxy/prod -> honeypot=false: the realm just created for this decoy is a
  #             canary (no users, events on), so it presents the exact same SSO
  #             surface as a real tenant and Keycloak records every attempt.
  if [[ "$role" == decoy && "$MODE" == local ]]; then
    args+=(--set honeypot=true)
  fi

  helm upgrade --install "$rel" "$CHART" "${args[@]}" >/dev/null || die "helm failed for $codename"
  kubectl label ns "$ns" "$LABEL" "castle.io/codename=$codename" "castle.io/role=$role" --overwrite >/dev/null
  [[ "$MODE" == local ]] || apply_idp_egress "$ns" "$codename"
  echo "  ok"
}

list(){
  printf '%-16s %-8s %-20s %-7s %s\n' CODENAME ROLE NAMESPACE READY URL
  kubectl get ns -l "$LABEL" -o jsonpath='{range .items[*]}{.metadata.labels.castle\.io/codename} {.metadata.labels.castle\.io/role} {.metadata.name}{"\n"}{end}' 2>/dev/null \
  | while read -r cn role ns; do
      [[ -n "$ns" ]] || continue
      local ready; ready="$(kubectl -n "$ns" get deploy -o jsonpath='{.items[0].status.readyReplicas}/{.items[0].status.replicas}' 2>/dev/null)"
      printf '%-16s %-8s %-20s %-7s %s\n' "$cn" "$role" "$ns" "${ready:-0/0}" "https://$(host_of "$cn")"
    done
}

deprovision(){
  if [[ "${1:-}" == "--all" ]]; then
    kubectl get ns -l "$LABEL" -o jsonpath='{range .items[*]}{.metadata.labels.castle\.io/codename}{"\n"}{end}' 2>/dev/null \
    | while read -r cn; do [[ -n "$cn" ]] && deprovision "$cn"; done
    return
  fi
  local codename="$1"; [[ -n "$codename" ]] || die "deprovision needs a <codename> or --all"
  local ns; ns="$(kubectl get ns -l "castle.io/codename=$codename" -o jsonpath='{.items[0].metadata.name}' 2>/dev/null)"
  [[ -n "$ns" ]] || die "no managed instance '$codename'"
  echo "deprovisioning '$codename' (ns $ns)"
  helm uninstall "castle-$codename" -n "$ns" >/dev/null 2>&1
  kubectl delete ns "$ns" --wait=false >/dev/null 2>&1
  # In proxy/prod the realm lives on the shared Keycloak, outside the namespace,
  # so deleting the namespace does not remove it. Leaving stale realms behind
  # would leak the count of past tenants and clutter the IdP.
  if [[ "$MODE" != local && -n "${KC_PUBLIC_URL:-}" ]]; then
    KC_URL="$KC_PUBLIC_URL" KC_REALM_PREFIX="$KC_REALM_PREFIX" \
      "$REALM_SH" delete "$codename" 2>/dev/null && echo "  realm removed" || true
  fi
  echo "  removed"
}

# allocate: pick a free codename, provision it, and print it. No client name is
# taken or stored — you record which client this codename belongs to in your own
# records, off this cluster. The printed codename is the only handle you need.
allocate(){
  local role="${1:-tenant}"
  local codename; codename="$(next_codename)"
  provision "$codename" "$role"
  echo "  allocated codename: $codename ($role)"
  echo "  record the client<->codename mapping in your own store; the cluster will not."
}

# pool: free vs allocated codenames, so it is obvious when the pre-issued supply
# is running low (and its certificates need topping up in bulk).
pool(){
  local total free_n alloc_n
  total="$(pool_codenames | wc -l)"
  alloc_n="$(allocated_codenames | wc -l)"
  free_n="$((total - alloc_n))"
  echo "codename pool: $free_n free / $total total ($alloc_n allocated)"
  echo "  allocated: $(allocated_codenames | paste -sd' ' -)"
  [[ "$free_n" -le 5 ]] && echo "  WARNING: pool nearly exhausted — add names and issue their certs in bulk" >&2
  return 0
}

usage(){
  cat >&2 <<'U'
usage: castlectl <command> [args]   (MODE=local|proxy|prod, default local)

  allocate [--decoy]                  pick a free codename + provision it (prints the codename)
  provision <codename> [--decoy]      provision a specific codename
  pool                                free vs allocated codenames
  list                                running instances + readiness
  deprovision <codename> | --all      remove instance(s) and realm(s)

The cluster only ever knows codenames. Record which client a codename belongs to
in your own store, off this cluster — see the header of this script.
U
  exit 1
}

# --decoy may appear before or after the positional arg; strip it out and set the role.
role=tenant; posargs=()
cmd="${1:-}"; shift || true
for a in "$@"; do
  if [[ "$a" == "--decoy" ]]; then role=decoy; else posargs+=("$a"); fi
done

case "$cmd" in
  allocate)     allocate "$role" ;;
  provision)    provision "${posargs[0]:-}" "$role" ;;
  pool)         pool ;;
  list)         list ;;
  deprovision)  deprovision "${posargs[0]:-}" ;;
  ''|-h|--help|help) usage ;;
  *) echo "castlectl: unknown command '$cmd'" >&2; usage ;;
esac
