#!/bin/sh
# Tests for shred.sh against the dev OpenBao (run inside the openbao container, Task 2).
# Root creates fixtures; shred.sh itself runs as dev-only-operator, like production.
set -eu
export BAO_ADDR="${BAO_ADDR:-http://127.0.0.1:8200}"
ROOT="${BAO_TOKEN:?root token}"
S="$(dirname "$0")/shred.sh"
Q=fau-keys-queue
fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }
mk() { BAO_TOKEN=$ROOT bao write -f "transit/keys/$1" >/dev/null; }
soft() { BAO_TOKEN=$ROOT bao read -field=soft_deleted "transit/keys/$1" 2>/dev/null || echo gone; }
run() { now="$1"; shift; BAO_TOKEN=dev-only-operator SHRED_NOW="$now" sh "$S" "$@"; }
# Root-level fixture helpers, for scenarios shred.sh itself would never produce.
root_soft_delete() { BAO_TOKEN=$ROOT bao delete "transit/keys/$1/soft-delete" >/dev/null; }
root_restore() { BAO_TOKEN=$ROOT bao write -f "transit/keys/$1/soft-delete-restore" >/dev/null; }
root_queue() { k="$1"; shift; BAO_TOKEN=$ROOT bao write "$Q/$k" "$@" >/dev/null; }
root_unqueue() { BAO_TOKEN=$ROOT bao delete "$Q/$1" >/dev/null 2>&1 || true; }
queued() { BAO_TOKEN=$ROOT bao read "$Q/$1" >/dev/null 2>&1; }

T=$(cat /proc/sys/kernel/random/uuid); D=$(cat /proc/sys/kernel/random/uuid)
NOW=1790500000                         # 2026-09-27T10:26:40Z
DAY=86400
mk "fau-$T-record"; mk "fau-$T-messages"; mk "fau-$T-doc-$D"; mk "fau-$T-chat-2026-09"
OTHER=$(cat /proc/sys/kernel/random/uuid); mk "fau-$OTHER-record"

run $NOW document "$T" "$D"
[ "$(soft fau-$T-doc-$D)" = "true" ] && [ "$(soft fau-$T-record)" = "false" ] || fail "document soft-deletes exactly one key"
pass "document"

run $NOW restore "fau-$T-doc-$D" 2>/dev/null
[ "$(soft fau-$T-doc-$D)" = "false" ] || fail "restore"
BAO_TOKEN=$ROOT bao read "fau-keys-queue/fau-$T-doc-$D" >/dev/null 2>&1 && fail "restore unqueues"
pass "restore"

run $NOW fau "$T"
for k in record messages "doc-$D" chat-2026-09; do [ "$(soft fau-$T-$k)" = "true" ] || fail "fau soft-deletes fau-$T-$k"; done
[ "$(soft fau-$OTHER-record)" = "false" ] || fail "fau leaves other FAUs alone"
pass "fau, including a young chat month (governed deletion, not expiry)"

run $((NOW + 6*DAY)) finalize
[ "$(soft fau-$T-record)" = "true" ] || fail "nothing destroyed before 7 days"
run $((NOW + 7*DAY)) finalize
[ "$(soft fau-$T-record)" = "gone" ] && [ "$(soft fau-$T-chat-2026-09)" = "gone" ] || fail "destroyed after 7 days"
[ "$(soft fau-$OTHER-record)" = "false" ] || fail "finalize leaves unqueued keys alone"
pass "finalize after 7 days (including a young chat month legitimately queued under reason=fau)"

C=$(cat /proc/sys/kernel/random/uuid)
mk "fau-$C-chat-2025-08"; mk "fau-$C-chat-2025-09"; mk "fau-$C-chat-2026-09"
run $NOW chat-expire                   # current UTC month 2026-09 (index diff: 2025-08 -> 13)
[ "$(soft fau-$C-chat-2025-08)" = "true" ] || fail "a month that ended 12+ months ago expires"
[ "$(soft fau-$C-chat-2025-09)" = "false" ] && [ "$(soft fau-$C-chat-2026-09)" = "false" ] || fail "younger months are kept"
pass "chat-expire"

# A queued key that someone restored by hand must not be destroyed by finalize.
R=$(cat /proc/sys/kernel/random/uuid); mk "fau-$R-record"
run $NOW fau "$R"
BAO_TOKEN=$ROOT bao write -f "transit/keys/fau-$R-record/soft-delete-restore" >/dev/null
run $((NOW + 8*DAY)) finalize
[ "$(soft fau-$R-record)" = "false" ] || fail "finalize destroyed a key that was restored out of band"
pass "finalize re-checks soft_deleted"

# --- Fix round 1 additions -------------------------------------------------

# (a) A young chat month can only be reached by hand-tampering the queue (chat-expire never
# selects one, and `fau` queues under reason=fau, already proven destroyed above). finalize must
# refuse it: exit 3, CRITICAL on stderr, key stays soft-deleted, entry stays queued.
Y=$(cat /proc/sys/kernel/random/uuid); YK="fau-$Y-chat-2026-09"
mk "$YK"
root_soft_delete "$YK"
root_queue "$YK" soft_deleted_at="$((NOW - 8*DAY))" reason=chat-expire
errout=$(run $NOW finalize 2>&1 1>/dev/null) && rc=0 || rc=$?
[ "$rc" = 3 ] || fail "finalize must exit 3 when it refuses a young chat month (rc=$rc)"
echo "$errout" | grep -q CRITICAL || fail "refusal logs CRITICAL on stderr: $errout"
[ "$(soft "$YK")" = "true" ] || fail "refused key stays soft-deleted"
queued "$YK" || fail "refused key stays queued"
pass "finalize refuses a young chat month queued under reason=chat-expire"
# Clean up so this fixture does not make a later run's finalize refuse (and exit 3) again.
root_restore "$YK"; root_unqueue "$YK"

# (c) A queue entry missing soft_deleted_at (a malformed/hand-edited entry) must not abort the
# sweep under set -e: it is skipped (crit-logged), and the sweep still reaches later entries.
M=$(cat /proc/sys/kernel/random/uuid); MK="fau-$M-record"
mk "$MK"; root_soft_delete "$MK"; root_queue "$MK" reason=fau   # no soft_deleted_at field
M2=$(cat /proc/sys/kernel/random/uuid); M2K="fau-$M2-record"
mk "$M2K"; run $NOW fau "$M2"
out=$(run $((NOW + 8*DAY)) finalize 2>&1) && rc=0 || rc=$?
[ "$rc" = 0 ] || fail "a malformed queue entry must not turn finalize's exit into non-zero by itself (rc=$rc): $out"
echo "$out" | grep -q CRITICAL || fail "malformed entry logs CRITICAL: $out"
[ "$(soft "$M2K")" = "gone" ] || fail "sweep continues past the malformed entry and destroys the next key"
[ "$(soft "$MK")" = "true" ] || fail "malformed entry's key is left alone, not destroyed"
queued "$MK" || fail "malformed entry stays queued for manual repair"
pass "finalize skips a queue entry missing soft_deleted_at instead of aborting the sweep"
root_restore "$MK"; root_unqueue "$MK"

# (b) Idempotency, for real: a second run over the SAME already-soft-deleted key, with a later
# clock, must exit 0, log a skip, and must not reset soft_deleted_at (no re-arming the 7-day clock).
I=$(cat /proc/sys/kernel/random/uuid); IK="fau-$I-record"
mk "$IK"
run $NOW fau "$I"
[ "$(soft "$IK")" = "true" ] || fail "idempotency fixture: first run soft-deletes"
at1=$(BAO_TOKEN=$ROOT bao read -field=soft_deleted_at "$Q/$IK")
out=$(run $((NOW + 2*DAY)) fau "$I") && rc=0 || rc=$?
[ "$rc" = 0 ] || fail "re-running fau over an already soft-deleted key must exit 0 (rc=$rc)"
echo "$out" | grep -q skipped || fail "re-run logs a skip line: $out"
at2=$(BAO_TOKEN=$ROOT bao read -field=soft_deleted_at "$Q/$IK")
[ "$at1" = "$at2" ] || fail "re-run must not reset soft_deleted_at (was $at1, now $at2)"
pass "idempotent: unchanged soft_deleted_at, exit 0, skip logged"

if run $NOW nonsense 2>/dev/null; then fail "unknown command must fail"; fi
pass "bad input"
echo "all shred.sh tests passed"
