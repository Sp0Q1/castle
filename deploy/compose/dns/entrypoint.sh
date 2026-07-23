#!/bin/sh
# Renders named.conf + the initial zone from the environment, then runs BIND in
# the foreground. Config is regenerated every start (picks up .env changes); the
# zone is seeded ONLY if absent, so restarts never clobber the live dynamic zone
# (its A records and the ACME journal live in the dns_data volume).
set -eu

: "${BASE_DOMAIN:?BASE_DOMAIN required}"
: "${PUBLIC_IP:?PUBLIC_IP required}"
: "${DDNS_TSIG_SECRET:?DDNS_TSIG_SECRET required}"
NS_NAME="${NS_NAME:-ns1.${BASE_DOMAIN}}"
TSIG_NAME="${TSIG_NAME:-castle-ddns}"

DATA=/var/lib/bind
ZONE="${DATA}/db.${BASE_DOMAIN}"

render() {
  sed -e "s#__BASE_DOMAIN__#${BASE_DOMAIN}#g" \
      -e "s#__PUBLIC_IP__#${PUBLIC_IP}#g" \
      -e "s#__NS_NAME__#${NS_NAME}#g" \
      -e "s#__TSIG_NAME__#${TSIG_NAME}#g" \
      -e "s#__TSIG_SECRET__#${DDNS_TSIG_SECRET}#g" \
      "$1"
}

mkdir -p "$DATA"
render /templates/named.conf.template > "${DATA}/named.conf"
[ -f "$ZONE" ] || render /templates/db.zone.template > "$ZONE"

# BIND writes the zone + journal here; it runs as the unprivileged bind user.
chown -R bind:bind "$DATA" 2>/dev/null || true

exec /usr/sbin/named -g -u bind -c "${DATA}/named.conf"
