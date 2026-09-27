#!/bin/sh
# FAU key deletion (docs/key-service-design.md §5). Runs as fau-keys-operator, which can
# destroy keys and never read data. See the usage line below.
#
# Exit codes (docs/key-service-operations.md §4):
#   0  done (keys skipped as already soft-deleted and queued, or gone, still count as done)
#   1  OpenBao error: unreachable, sealed, token rejected, permission denied, or any failed
#      read or write. Nothing is ever treated as "empty" or "gone" unless OpenBao said so.
#   2  bad input (usage)
#   3  finalize refused to destroy a chat key whose month is younger than 12 months and whose
#      queue reason is not `fau` (spec §5: refuse and log at critical)
#   4  `fau` matched no keys at all: a wrong tenant id, or an FAU that never stored content
# Exits 1, 3 and 4 log a `shred: CRITICAL` line, which KeyServiceShredCritical alerts on.
set -eu
WINDOW=$((7 * 24 * 3600))              # ADR-003 decision 7
Q=fau-keys-queue

now() { echo "${SHRED_NOW:-$(date -u +%s)}"; }
log() { echo "shred: $*"; }
crit() { echo "shred: CRITICAL $*" >&2; }
fatal() { crit "$*"; exit 1; }
usage() { echo "usage: shred.sh fau <tenant> | document <tenant> <document> | chat-expire | restore <key> | finalize" >&2; exit 2; }
is_uuid() { echo "$1" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; }
oneline() { tr '\n\t' '  ' | cut -c1-400; }   # bao's error text: path and status, never a token
# The jobs run with a read-only root filesystem, so no temp file for stderr: a command whose
# stdout is not needed captures stderr alone (mut, reach); a read that failed is repeated once
# with stderr captured, to classify it (rd, list).
mut() { err=$(bao "$@" 2>&1 >/dev/null) || fatal "OpenBao refused: bao $*: $(echo "$err" | oneline)"; }
# Once at start: a bad address, a sealed server or a rejected token fails here, not later as
# an "empty" list. lookup-self is in OpenBao's default policy, which every operator token has.
reach() { err=$(bao token lookup 2>&1 >/dev/null) || fatal "OpenBao is unreachable, sealed, or rejected the token: $(echo "$err" | oneline)"; }
# list <path>: prints one name per line. Only OpenBao's own empty answer (exit 2 with `{}` on
# stdout, verified against 2.7.0) is "nothing"; any other failure exits 1. Call it as an
# assignment (`names=$(list …) || exit 1`): an exit inside a `for … in $(…)` would be lost.
list() {
  if out=$(bao list -format=json "$1" 2>/dev/null); then
    echo "$out" | tr -d '[]", ' | sed '/^$/d'
  elif [ "$out" != "{}" ]; then
    fatal "cannot list $1: $(bao list -format=json "$1" 2>&1 >/dev/null | oneline)"
  fi
}
# rd <path> [field]: sets RD and returns 0; returns 1 if nothing is stored at the path, 2 if
# the entry has no such field. Any other failure (denied, unreachable, sealed) exits 1.
rd() {
  if RD=$(bao read ${2:+-field=$2} "$1" 2>/dev/null); then return 0; fi
  err=$(bao read ${2:+-field=$2} "$1" 2>&1 >/dev/null) && fatal "read of $1 failed once, then succeeded; not trusting it" || true
  case "$err" in
    "No value found at "*) return 1 ;;
    "Field \""*"\" not present in secret"*) return 2 ;;
  esac
  fatal "cannot read $1: $(echo "$err" | oneline)"
}
month_index() { y=${1%-*}; m=${1#*-}; m=${m#0}; echo $((y * 12 + m - 1)); }
# Shared rule (matches ChatMonth::index()): true if chat key $1's month index is fewer than 13
# months behind the current UTC month index $2 (i.e. the month ended less than 12 months ago).
chat_is_young() { m="${1##*-chat-}"; [ $(( $2 - $(month_index "$m") )) -lt 13 ]; }

soft() {
  # $2 is the queue reason (fau | document | chat-expire): finalize uses it to tell a governed
  # FAU deletion, which may destroy a young chat month, from an expiry, which may not.
  rd "transit/keys/$1" soft_deleted && rc=0 || rc=$?
  [ "$rc" != 1 ] || { log "skipped $1 (gone)"; return 0; }
  [ "$rc" = 0 ] || fatal "transit/keys/$1 has no soft_deleted field"
  if [ "$RD" = true ]; then
    # Idempotent: already soft-deleted and queued (an earlier run, or the monthly job again)
    # is skipped, keeping the original soft_deleted_at so the 7-day clock is not re-armed.
    if rd "$Q/$1"; then log "skipped $1 (already soft-deleted and queued)"; return 0; fi
    mut write "$Q/$1" soft_deleted_at="$(now)" reason="$2"
    crit "re-queued $1: it was soft-deleted but not queued (a run that stopped between the two writes?)"
    return 0
  fi
  [ "$RD" = false ] || fatal "transit/keys/$1 has soft_deleted=$RD"
  # Soft delete comes before the queue write. A crash between the two leaves a key soft-deleted
  # but unqueued: inert, restorable, and re-queued by the next run (above). The opposite order
  # could leave an active key queued for destruction.
  mut delete "transit/keys/$1/soft-delete"
  mut write "$Q/$1" soft_deleted_at="$(now)" reason="$2"
  log "soft-deleted $1"
}

cmd="${1:-}"; [ -n "$cmd" ] || usage; shift
case "$cmd" in
  fau)
    t="${1:-}"; is_uuid "$t" || usage
    reach
    names=$(list transit/keys) || exit 1
    n=0
    for k in $names; do case "$k" in "fau-$t-"*) soft "$k" fau; n=$((n + 1)) ;; esac; done
    [ "$n" -gt 0 ] || { crit "fau $t matched no keys (wrong tenant id?); nothing was deleted"; exit 4; }
    ;;
  document)
    t="${1:-}"; d="${2:-}"; is_uuid "$t" && is_uuid "$d" || usage
    reach
    soft "fau-$t-doc-$d" document
    ;;
  chat-expire)
    reach
    cur=$(month_index "$(date -u -d "@$(now)" +%Y-%m)")
    names=$(list transit/keys) || exit 1
    for k in $names; do
      case "$k" in fau-*-chat-[0-9][0-9][0-9][0-9]-[0-9][0-9]) ;; *) continue ;; esac
      chat_is_young "$k" "$cur" || soft "$k" chat-expire
    done
    ;;
  restore)
    k="${1:-}"; case "$k" in fau-*) ;; *) usage ;; esac
    reach
    mut write -f "transit/keys/$k/soft-delete-restore"
    mut delete "$Q/$k"                 # deleting a missing KV v1 entry succeeds
    crit "restored $k"
    ;;
  finalize)
    reach
    REFUSED=0
    cur=$(month_index "$(date -u -d "@$(now)" +%Y-%m)")
    names=$(list "$Q") || exit 1
    for k in $names; do
      rd "$Q/$k" soft_deleted_at && rc=0 || rc=$?
      case "$rc" in
        1) continue ;;                 # unqueued since the list (a restore running alongside)
        2) crit "$k has no soft_deleted_at in the queue (malformed entry); skipped"; continue ;;
      esac
      at=$RD
      # busybox sh evaluates a non-numeric word in $(( )) as an unset variable, i.e. 0, which
      # would make the entry look 56 years old: validate before any arithmetic.
      case "$at" in ''|*[!0-9]*) crit "$k has a non-numeric soft_deleted_at in the queue (malformed entry); skipped"; continue ;; esac
      reason=""                        # a missing reason counts as not-fau
      if rd "$Q/$k" reason; then reason=$RD; fi
      [ $(( $(now) - at )) -ge $WINDOW ] || continue
      rd "transit/keys/$k" soft_deleted && rc=0 || rc=$?
      case "$rc" in 0) state=$RD ;; 1) state=gone ;; *) fatal "transit/keys/$k has no soft_deleted field" ;; esac
      if [ "$state" != "true" ]; then
        crit "$k is queued but not soft-deleted (state=$state: restored out of band, or destroyed already?); unqueued, not destroyed"
        mut delete "$Q/$k"
        continue
      fi
      case "$k" in
        fau-*-chat-[0-9][0-9][0-9][0-9]-[0-9][0-9])
          if [ "$reason" != "fau" ] && chat_is_young "$k" "$cur"; then
            crit "refused $k: chat month is younger than 12 months (reason=${reason:-none})"
            REFUSED=1
            continue
          fi
          ;;
      esac
      mut write "transit/keys/$k/config" deletion_allowed=true
      mut delete "transit/keys/$k"
      mut delete "$Q/$k"
      log "destroyed $k"
    done
    [ "$REFUSED" = 0 ] || exit 3
    ;;
  *) usage ;;
esac
