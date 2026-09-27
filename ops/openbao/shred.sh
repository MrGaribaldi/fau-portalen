#!/bin/sh
# FAU key deletion (docs/key-service-design.md §5). Runs as fau-keys-operator, which can
# destroy keys and never read data. See the usage line below.
set -eu
WINDOW=$((7 * 24 * 3600))              # ADR-003 decision 7
Q=fau-keys-queue

now() { echo "${SHRED_NOW:-$(date -u +%s)}"; }
log() { echo "shred: $*"; }
crit() { echo "shred: CRITICAL $*" >&2; }
usage() { echo "usage: shred.sh fau <tenant> | document <tenant> <document> | chat-expire | restore <key> | finalize" >&2; exit 2; }
is_uuid() { echo "$1" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; }
# `bao list` prints a JSON array; an empty mount is an error, which means "nothing".
list() { bao list -format=json "$1" 2>/dev/null | tr -d '[]", ' | sed '/^$/d' || true; }
month_index() { y=${1%-*}; m=${1#*-}; m=${m#0}; echo $((y * 12 + m - 1)); }

soft() {
  # Idempotent: a key already soft-deleted (an earlier run, or the monthly job again) is skipped.
  [ "$(bao read -field=soft_deleted "transit/keys/$1" 2>/dev/null || echo gone)" = "false" ] || { log "skipped $1 (already soft-deleted or gone)"; return 0; }
  bao delete "transit/keys/$1/soft-delete" >/dev/null      # first: see the ordering rule
  bao write "$Q/$1" soft_deleted_at="$(now)" >/dev/null
  log "soft-deleted $1"
}

cmd="${1:-}"; [ -n "$cmd" ] || usage; shift
case "$cmd" in
  fau)
    t="${1:-}"; is_uuid "$t" || usage
    for k in $(list transit/keys); do case "$k" in "fau-$t-"*) soft "$k" ;; esac; done
    ;;
  document)
    t="${1:-}"; d="${2:-}"; is_uuid "$t" && is_uuid "$d" || usage
    soft "fau-$t-doc-$d"
    ;;
  chat-expire)
    cur=$(month_index "$(date -u -d "@$(now)" +%Y-%m)")
    for k in $(list transit/keys); do
      m="${k##*-chat-}"
      case "$k" in fau-*-chat-[0-9][0-9][0-9][0-9]-[0-9][0-9]) ;; *) continue ;; esac
      if [ $((cur - $(month_index "$m"))) -ge 13 ]; then soft "$k"; fi
    done
    ;;
  restore)
    k="${1:-}"; case "$k" in fau-*) ;; *) usage ;; esac
    bao write -f "transit/keys/$k/soft-delete-restore" >/dev/null
    bao delete "$Q/$k" >/dev/null 2>&1 || true
    crit "restored $k"
    ;;
  finalize)
    for k in $(list "$Q"); do
      at=$(bao read -field=soft_deleted_at "$Q/$k")
      [ $(( $(now) - at )) -ge $WINDOW ] || continue
      if [ "$(bao read -field=soft_deleted "transit/keys/$k" 2>/dev/null || echo gone)" != "true" ]; then
        crit "$k is queued but not soft-deleted (restored out of band?); unqueued, not destroyed"
        bao delete "$Q/$k" >/dev/null
        continue
      fi
      bao write "transit/keys/$k/config" deletion_allowed=true >/dev/null
      bao delete "transit/keys/$k" >/dev/null
      bao delete "$Q/$k" >/dev/null
      log "destroyed $k"
    done
    ;;
  *) usage ;;
esac
