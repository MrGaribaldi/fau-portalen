#!/bin/sh
# Tests for shred.sh against the dev OpenBao (run inside the openbao container, Task 2).
# Root creates fixtures; shred.sh itself runs as dev-only-operator, like production.
set -eu
export BAO_ADDR="${BAO_ADDR:-http://127.0.0.1:8200}"
ROOT="${BAO_TOKEN:?root token}"
S="$(dirname "$0")/shred.sh"
fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }
mk() { BAO_TOKEN=$ROOT bao write -f "transit/keys/$1" >/dev/null; }
soft() { BAO_TOKEN=$ROOT bao read -field=soft_deleted "transit/keys/$1" 2>/dev/null || echo gone; }
run() { now="$1"; shift; BAO_TOKEN=dev-only-operator SHRED_NOW="$now" sh "$S" "$@"; }

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
pass "finalize after 7 days"

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

run $NOW fau "$T" >/dev/null           # a second run over already soft-deleted keys succeeds
pass "idempotent"
if run $NOW nonsense 2>/dev/null; then fail "unknown command must fail"; fi
pass "bad input"
echo "all shred.sh tests passed"
