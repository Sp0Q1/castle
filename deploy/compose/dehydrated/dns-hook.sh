#!/usr/bin/env bash
# dehydrated DNS-01 hook for castle's self-hosted authoritative DNS (Option 3).
#
# It publishes the _acme-challenge TXT record by sending an RFC 2136 dynamic
# update (nsupdate) to the nameserver castle runs itself — authenticated with a
# TSIG key, NOT a DNS-provider API key. The record is added just long enough for
# Let's Encrypt to read it and removed straight after, so no _acme-challenge
# record stands in the zone between issuances.
#
# dehydrated invokes this as:
#   dns-hook.sh deploy_challenge <domain> <token_filename> <token_value>
#   dns-hook.sh clean_challenge  <domain> <token_filename> <token_value>
# and for several other lifecycle events we don't need (cert install is done by
# castlectl, which copies the results into Caddy's cert dir).
#
# Environment (exported by castlectl when it runs dehydrated):
#   DNS_SERVER     where to send the update      (default 127.0.0.1)
#   DNS_PORT       its port                       (default 53)
#   TSIG_KEYFILE   nsupdate key file (bind-style key{} block)   (required)
#   ACME_TXT_TTL   TTL for the challenge record   (default 30)
set -euo pipefail

DNS_SERVER="${DNS_SERVER:-127.0.0.1}"
DNS_PORT="${DNS_PORT:-53}"
TTL="${ACME_TXT_TTL:-30}"
: "${TSIG_KEYFILE:?dns-hook: TSIG_KEYFILE is required}"

_nsupdate() {
  nsupdate -k "$TSIG_KEYFILE" <<EOF
server ${DNS_SERVER} ${DNS_PORT}
$1
send
EOF
}

deploy_challenge() {
  local domain="$1" value="$3" rr="_acme-challenge.${1}."
  # delete-then-add so a retried issuance replaces rather than stacks records.
  _nsupdate "update delete ${rr} TXT
update add ${rr} ${TTL} IN TXT \"${value}\""
  # Don't hand control back to Let's Encrypt until our own authoritative server
  # actually answers with the token — otherwise validation can race the update.
  local i
  for i in $(seq 1 20); do
    if dig +short "@${DNS_SERVER}" -p "${DNS_PORT}" TXT "$rr" | grep -qF "$value"; then
      return 0
    fi
    sleep 1
  done
  echo "dns-hook: TXT ${rr} not visible on ${DNS_SERVER}:${DNS_PORT} after 20s" >&2
  return 1
}

clean_challenge() {
  local rr="_acme-challenge.${1}."
  _nsupdate "update delete ${rr} TXT"
}

case "${1:-}" in
  deploy_challenge) shift; deploy_challenge "$@" ;;
  clean_challenge)  shift; clean_challenge "$@" ;;
  # Everything else (deploy_cert, unchanged_cert, startup_hook, exit_hook,
  # invalid_challenge, request_failure, …) is a no-op for us.
  *) : ;;
esac
