#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Produce a committable SealedSecret bundle for a tenant's secrets, so no
# plaintext secret ever touches Helm values or shell history.
#
#   ./seal-tenant.sh <release> <namespace> [--proxy] [--internal-db] > sealed-<release>.yaml
#   kubectl apply -f sealed-<release>.yaml        # controller creates <release>-app (+ -db)
#   helm upgrade --install <release> ../tenant -n <namespace> --set secrets.external=true ...
#
# Requires `kubeseal` + kubectl access to the target cluster: the SealedSecret is
# encrypted to THAT cluster's controller key and only decrypts there, so the
# output is safe to commit to git.
#
# Real values come from the environment (so they don't land in CLI history);
# generated fresh if unset:
#   DATABASE_URL, JWT_SECRET, CASTLE_PROXY_SECRET,
#   OAUTH2_CLIENT_SECRET (required with --proxy), OAUTH2_COOKIE_SECRET,
#   DB_PASSWORD (required with --internal-db)
# ---------------------------------------------------------------------------
set -euo pipefail
rel="${1:?usage: seal-tenant.sh <release> <namespace> [--proxy] [--internal-db]}"
ns="${2:?namespace required}"; shift 2
proxy=false; internal_db=false
for a in "$@"; do case "$a" in --proxy) proxy=true ;; --internal-db) internal_db=true ;; esac; done
seal(){ kubeseal --controller-namespace "${SS_NS:-kube-system}" --format yaml; }

app=(--from-literal=DATABASE_URL="${DATABASE_URL:-sqlite:///app/uploads/castle.sqlite?mode=rwc}"
     --from-literal=JWT_SECRET="${JWT_SECRET:-$(openssl rand -base64 48 | tr -d '\n')}"
     --from-literal=CASTLE_PROXY_SECRET="${CASTLE_PROXY_SECRET:-}")
if $proxy; then
  app+=(--from-literal=OAUTH2_PROXY_CLIENT_SECRET="${OAUTH2_CLIENT_SECRET:?set OAUTH2_CLIENT_SECRET for --proxy}"
        --from-literal=OAUTH2_PROXY_COOKIE_SECRET="${OAUTH2_COOKIE_SECRET:-$(openssl rand -hex 16)}")
fi
kubectl create secret generic "${rel}-app" -n "$ns" "${app[@]}" --dry-run=client -o yaml | seal

if $internal_db; then
  echo "---"
  kubectl create secret generic "${rel}-db" -n "$ns" \
    --from-literal=POSTGRES_PASSWORD="${DB_PASSWORD:?set DB_PASSWORD for --internal-db}" \
    --dry-run=client -o yaml | seal
fi
