#!/bin/sh
# Configure OpenBao for FAU (docs/key-service-design.md §4.3). Idempotent: safe to re-run
# after any change to this file or the policies.
#
# Usage: BAO_ADDR=... BAO_TOKEN=<root or setup token> configure.sh dev|cluster
#   dev      compose and tests: token auth, fixed dev tokens (never outside compose)
#   cluster  Kubernetes auth roles for the app and the deletion job
set -eu
MODE="${1:?usage: configure.sh dev|cluster}"
HERE="$(cd "$(dirname "$0")" && pwd)"

has() { bao "$1" list -format=json 2>/dev/null | grep -q "\"$2/\""; }

has secrets transit || bao secrets enable transit
has secrets fau-keys-queue || bao secrets enable -path=fau-keys-queue -version=1 kv
# The stdout audit device (HMACs every value; key material and plaintext never reach the
# log, §4.4) is declared in the server configuration, not here: OpenBao refuses to create
# audit devices via the API by default, so dev-server.hcl and Task 9's helm-values.yaml each
# carry the same `audit "file" "stdout"` stanza instead.

bao policy write fau-app "$HERE/policies/fau-app.hcl"
bao policy write fau-keys-operator "$HERE/policies/fau-keys-operator.hcl"

# Replaces the hand-written design's lease ceiling (§4.3); tuned under #3442.
bao write sys/quotas/rate-limit/fau-transit path=transit/ rate=50 interval=1s >/dev/null

case "$MODE" in
  dev)
    for t in "dev-only-app:fau-app" "dev-only-operator:fau-keys-operator"; do
      id="${t%%:*}"; policy="${t#*:}"
      bao token lookup "$id" >/dev/null 2>&1 || bao token create -id="$id" -policy="$policy" -period=768h -orphan >/dev/null
    done
    ;;
  cluster)
    has auth kubernetes || bao auth enable kubernetes
    bao write auth/kubernetes/config kubernetes_host="https://kubernetes.default.svc" >/dev/null
    bao write auth/kubernetes/role/fau-app \
      bound_service_account_names=fau-app bound_service_account_namespaces=fau-app \
      policies=fau-app token_ttl=1h token_max_ttl=24h >/dev/null
    bao write auth/kubernetes/role/fau-keys-operator \
      bound_service_account_names=fau-keys-operator bound_service_account_namespaces=openbao \
      policies=fau-keys-operator token_ttl=15m token_max_ttl=1h >/dev/null
    ;;
  *) echo "configure.sh: unknown mode $MODE" >&2; exit 2 ;;
esac
echo "configure.sh: done ($MODE)"
