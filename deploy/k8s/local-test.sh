#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# Local end-to-end test of the castle multi-tenant + canary-swarm design on a
# throwaway kind cluster with Calico (so NetworkPolicies actually enforce).
#
#   ./deploy/k8s/local-test.sh          # create cluster, deploy, run checks
#   ./deploy/k8s/local-test.sh down     # tear the cluster down
#   REBUILD=1 ./deploy/k8s/local-test.sh   # force a fresh image build
#
# Needs: kind, kubectl, helm, and docker OR podman. No cloud, no DNS. Routing is
# tested over HTTPS (the chart forces TLS) using ingress-nginx's default cert
# (-k). Run from the repo root.
#
# NOTE: assumes the kind node can pull public images. In a network-restricted
# environment, pre-pull Calico/ingress on the host and `kind load` them first.
# ---------------------------------------------------------------------------
set -uo pipefail
cd "$(dirname "$0")/../.."                       # repo root
CLUSTER=castle-local
CAL=v3.28.2
ING=controller-v1.11.3
HTTP_PORT=8081
HTTPS_PORT=8443

# --- pick a container engine (image name must match what the chart requests) --
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  ENGINE=docker; IMG_REPO=castle                 # docker.io/library/castle
elif command -v podman >/dev/null 2>&1; then
  ENGINE=podman; export KIND_EXPERIMENTAL_PROVIDER=podman
  IMG_REPO=localhost/castle                       # podman prefixes localhost/
else
  echo "need docker or podman"; exit 1
fi
IMG="$IMG_REPO:local"
for t in kind kubectl helm; do command -v $t >/dev/null || { echo "missing: $t"; exit 1; }; done

step(){ printf '\n\033[1;36m== %s\033[0m\n' "$*"; }
H(){ echo "$1.127.0.0.1.nip.io"; }

if [[ "${1:-up}" == "down" ]]; then
  step "deleting cluster $CLUSTER"; kind delete cluster --name "$CLUSTER"; exit 0
fi

# --- 1. cluster (default CNI off so Calico can enforce) ---------------------
if ! kind get clusters 2>/dev/null | grep -qx "$CLUSTER"; then
  step "creating kind cluster '$CLUSTER'"
  kind create cluster --name "$CLUSTER" --config - <<EOF
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
networking:
  disableDefaultCNI: true
  podSubnet: "192.168.0.0/16"
nodes:
  - role: control-plane
    kubeadmConfigPatches:
      - |
        kind: InitConfiguration
        nodeRegistration:
          kubeletExtraArgs: { node-labels: "ingress-ready=true" }
    extraPortMappings:
      - { containerPort: 80,  hostPort: ${HTTP_PORT},  protocol: TCP }
      - { containerPort: 443, hostPort: ${HTTPS_PORT}, protocol: TCP }
EOF
else
  step "cluster '$CLUSTER' already exists — reusing"
fi
kubectl config use-context "kind-$CLUSTER" >/dev/null

# --- 2. Calico --------------------------------------------------------------
step "installing Calico ($CAL)"
kubectl apply -f "https://raw.githubusercontent.com/projectcalico/calico/${CAL}/manifests/calico.yaml" >/dev/null
kubectl wait --for=condition=Ready node --all --timeout=300s

# --- 3. ingress-nginx -------------------------------------------------------
step "installing ingress-nginx ($ING)"
kubectl apply -f "https://raw.githubusercontent.com/kubernetes/ingress-nginx/${ING}/deploy/static/provider/kind/deploy.yaml" >/dev/null
kubectl -n ingress-nginx wait --for=condition=Available deploy/ingress-nginx-controller --timeout=240s

# --- 4. build + load the castle image --------------------------------------
NEED_BUILD=0
[[ "${REBUILD:-0}" == "1" ]] && NEED_BUILD=1
$ENGINE image inspect "$IMG" >/dev/null 2>&1 || NEED_BUILD=1
if [[ $NEED_BUILD == 1 ]]; then
  step "building $IMG (Rust + SPA multi-stage — first build takes a few minutes)"
  $ENGINE build -t "$IMG" -f Dockerfile . || { echo "image build failed"; exit 1; }
fi
step "loading $IMG into the cluster"
if [[ "$ENGINE" == "docker" ]]; then
  kind load docker-image "$IMG" --name "$CLUSTER"
else
  tar="$(mktemp --tmpdir=/tmp castleXXXX.tar)"; podman save "$IMG" -o "$tar"
  kind load image-archive "$tar" --name "$CLUSTER"; rm -f "$tar"
fi

# --- 5. deploy 2 tenants + 1 decoy (all via the chart, local mode) ----------
deploy(){  # $1=release $2=namespace $3=codename
  helm upgrade --install "$1" deploy/k8s/tenant \
    --namespace "$2" --create-namespace \
    --set codename="$3" --set baseDomain=127.0.0.1.nip.io \
    --set image.repository="$IMG_REPO" --set image.tag=local --set image.pullPolicy=IfNotPresent \
    --set oauth2Proxy.enabled=false --set runtimeClassName="" \
    --set database.internal=false \
    --set database.externalUrl='sqlite:///app/uploads/castle.sqlite?mode=rwc' \
    --set keycloak.issuerUrl=http://unused/realms/x \
    --set secrets.jwtSecret="$(openssl rand -base64 48 | tr -d '\n')" >/dev/null
}
step "deploying tenants (ironclad, thunderbolt) + decoy (nightfall)"
deploy castle-ironclad    tenant-ironclad    ironclad
deploy castle-thunderbolt tenant-thunderbolt thunderbolt
deploy castle-nightfall   castle-honeypot    nightfall
for ns in tenant-ironclad tenant-thunderbolt castle-honeypot; do
  d=$(kubectl -n "$ns" get deploy -o name | head -1)
  kubectl -n "$ns" rollout status "$d" --timeout=180s
done

# --- 6. checks --------------------------------------------------------------
get(){ curl -sk -o /dev/null -w '%{http_code}' --max-time 8 --resolve "$(H $1):${HTTPS_PORT}:127.0.0.1" "https://$(H $1):${HTTPS_PORT}/probe-$1"; }
step "TEST 1 — routing: each subdomain reaches its own backend"
for cn in ironclad thunderbolt nightfall; do echo "  https://$(H $cn)/probe-$cn -> HTTP $(get $cn)"; done
echo "  upstream pod per host (from ingress access log):"
kubectl -n ingress-nginx logs deploy/ingress-nginx-controller 2>/dev/null \
  | grep -oE 'probe-(ironclad|thunderbolt|nightfall).*192\.168\.[0-9.]+:5150' \
  | sed -E 's/(probe-[a-z]+).*(192\.168\.[0-9.]+:5150)/    \1 -> \2/' | sort -u

step "TEST 2 — isolation: cross-tenant blocked, allowed path works, A/B proof"
IPT=$(kubectl -n tenant-thunderbolt get pod -l app=castle-thunderbolt -o jsonpath='{.items[0].status.podIP}')
conn(){ kubectl -n "$1" exec deploy/"$2" -- timeout 5 bash -c "exec 3<>/dev/tcp/$3/5150 && echo CONNECTED" 2>/dev/null; }
echo -n "  ironclad -> thunderbolt:5150   : "; [[ "$(conn tenant-ironclad castle-ironclad "$IPT")" == CONNECTED ]] && echo "CONNECTED (unexpected!)" || echo "blocked ✓"
echo -n "  ironclad -> cluster DNS (allow): "; kubectl -n tenant-ironclad exec deploy/castle-ironclad -- timeout 5 getent hosts kubernetes.default.svc.cluster.local >/dev/null 2>&1 && echo "resolves ✓" || echo "FAILED"
echo "  A/B: adding a temporary allow policy…"
kubectl apply -f - >/dev/null 2>&1 <<EOF
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: tmp-allow-egress, namespace: tenant-ironclad }
spec: { podSelector: { matchLabels: { app: castle-ironclad } }, policyTypes: [Egress],
  egress: [ { to: [ { namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: tenant-thunderbolt } } } ], ports: [ { protocol: TCP, port: 5150 } ] } ] }
---
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata: { name: tmp-allow-ingress, namespace: tenant-thunderbolt }
spec: { podSelector: { matchLabels: { app: castle-thunderbolt } }, policyTypes: [Ingress],
  ingress: [ { from: [ { namespaceSelector: { matchLabels: { kubernetes.io/metadata.name: tenant-ironclad } } } ], ports: [ { protocol: TCP, port: 5150 } ] } ] }
EOF
sleep 4
echo -n "  ironclad -> thunderbolt (now)  : "; [[ "$(conn tenant-ironclad castle-ironclad "$IPT")" == CONNECTED ]] && echo "CONNECTED ✓ (policy was the enforcer)" || echo "still blocked (unexpected)"
kubectl -n tenant-ironclad delete netpol tmp-allow-egress >/dev/null 2>&1
kubectl -n tenant-thunderbolt delete netpol tmp-allow-ingress >/dev/null 2>&1

step "TEST 3 — detection: hits on the decoy are recorded (the CanaryTouched signal)"
echo "  ingress-logged requests to decoy 'nightfall': $(kubectl -n ingress-nginx logs deploy/ingress-nginx-controller 2>/dev/null | grep -c 'probe-nightfall')"

step "done — explore it yourself (ingress-nginx default cert, so -k)"
cat <<EOF
  curl -k --resolve $(H ironclad):${HTTPS_PORT}:127.0.0.1  https://$(H ironclad):${HTTPS_PORT}/
  curl -k --resolve $(H nightfall):${HTTPS_PORT}:127.0.0.1 https://$(H nightfall):${HTTPS_PORT}/   # decoy
  kubectl get pods -A | grep -E 'tenant-|honeypot'
  ./deploy/k8s/local-test.sh down     # tear down
EOF
