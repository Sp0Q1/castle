#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# castlectl — minimal per-client instance provisioner (the seed of the control
# plane). Wraps helm + kubectl so managing tenants is one command instead of a
# hand-run helm install + label dance.
#
#   castlectl provision <codename> [--decoy]   # stamp out a tenant (or a decoy)
#   castlectl list                             # show all managed instances
#   castlectl deprovision <codename>           # remove one
#   castlectl deprovision --all                # remove every managed instance
#
# Config via env (local defaults shown):
#   IMG_REPO=castle  IMG_TAG=local  BASE_DOMAIN=127.0.0.1.nip.io
#   MODE=local  (local = built-in login + sqlite + no kata; prod would add
#                proxy auth + per-tenant Postgres + a Keycloak realm)
#
# The codename<->real-client mapping is deliberately NOT stored here — in prod
# that lives in the control plane's secret store. We only label with the codename.
# ---------------------------------------------------------------------------
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
CHART="$HERE/tenant"
IMG_REPO="${IMG_REPO:-castle}"
IMG_TAG="${IMG_TAG:-local}"
BASE_DOMAIN="${BASE_DOMAIN:-127.0.0.1.nip.io}"
MODE="${MODE:-local}"
LABEL="castle.io/managed=true"

die(){ echo "castlectl: $*" >&2; exit 1; }
ns_of(){ [[ "$2" == decoy ]] && echo "decoy-$1" || echo "tenant-$1"; }

provision(){
  local codename="$1" role="${2:-tenant}"
  [[ -n "$codename" ]] || die "provision needs a <codename>"
  local ns; ns="$(ns_of "$codename" "$role")"
  echo "provisioning $role '$codename' -> $codename.$BASE_DOMAIN (ns $ns)"
  local args=(
    --namespace "$ns" --create-namespace
    --set codename="$codename" --set baseDomain="$BASE_DOMAIN"
    --set image.repository="$IMG_REPO" --set image.tag="$IMG_TAG" --set image.pullPolicy=IfNotPresent
    --set secrets.jwtSecret="$(openssl rand -base64 48 | tr -d '\n')"
  )
  if [[ "$MODE" == local ]]; then
    # local: built-in login (no sidecar => jwt), sqlite, no kata.
    args+=(--set oauth2Proxy.enabled=false --set runtimeClassName=""
           --set database.internal=false
           --set database.externalUrl='sqlite:///app/uploads/castle.sqlite?mode=rwc'
           --set keycloak.issuerUrl=http://unused/realms/x)
  fi
  helm upgrade --install "castle-$codename" "$CHART" "${args[@]}" >/dev/null || die "helm failed for $codename"
  kubectl label ns "$ns" "$LABEL" "castle.io/codename=$codename" "castle.io/role=$role" --overwrite >/dev/null
  echo "  ok"
}

list(){
  printf '%-16s %-8s %-20s %-7s %s\n' CODENAME ROLE NAMESPACE READY URL
  kubectl get ns -l "$LABEL" -o jsonpath='{range .items[*]}{.metadata.labels.castle\.io/codename} {.metadata.labels.castle\.io/role} {.metadata.name}{"\n"}{end}' 2>/dev/null \
  | while read -r cn role ns; do
      [[ -n "$ns" ]] || continue
      local ready; ready="$(kubectl -n "$ns" get deploy -o jsonpath='{.items[0].status.readyReplicas}/{.items[0].status.replicas}' 2>/dev/null)"
      printf '%-16s %-8s %-20s %-7s %s\n' "$cn" "$role" "$ns" "${ready:-0/0}" "https://$cn.$BASE_DOMAIN"
    done
}

deprovision(){
  if [[ "${1:-}" == "--all" ]]; then
    kubectl get ns -l "$LABEL" -o jsonpath='{range .items[*]}{.metadata.labels.castle\.io/codename} {.metadata.labels.castle\.io/role}{"\n"}{end}' 2>/dev/null \
    | while read -r cn role; do [[ -n "$cn" ]] && deprovision "$cn"; done
    return
  fi
  local codename="$1"; [[ -n "$codename" ]] || die "deprovision needs a <codename> or --all"
  local ns; ns="$(kubectl get ns -l "castle.io/codename=$codename" -o jsonpath='{.items[0].metadata.name}' 2>/dev/null)"
  [[ -n "$ns" ]] || die "no managed instance '$codename'"
  echo "deprovisioning '$codename' (ns $ns)"
  helm uninstall "castle-$codename" -n "$ns" >/dev/null 2>&1
  kubectl delete ns "$ns" --wait=false >/dev/null 2>&1
  echo "  removed"
}

cmd="${1:-}"; shift || true
case "$cmd" in
  provision)
    codename="${1:-}"; role=tenant
    [[ "${2:-}" == "--decoy" || "${1:-}" == "--decoy" ]] && role=decoy
    [[ "$codename" == "--decoy" ]] && codename="${2:-}"
    provision "$codename" "$role" ;;
  list)         list ;;
  deprovision)  deprovision "${1:-}" ;;
  *) echo "usage: castlectl {provision <codename> [--decoy] | list | deprovision <codename>|--all}"; exit 1 ;;
esac
