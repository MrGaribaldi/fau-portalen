#!/usr/bin/env bash
# Sync FAU-owned Terraform source files from the workspace into the private runtime root.
#
#   /workspace/infrastructure/<stage>   source of truth for configuration (persists in the repo)
#   /infra-runtime/infrastructure/<stage>   where Terraform actually runs (state, credentials, keys)
#
# Only the files named in MANAGED_FILES are ever written. State, lock files, .terraform/,
# .config/, credentials and anything Terraform generates are never touched or deleted:
# this script only ever copies the listed sources one way, workspace -> runtime.
#
# Usage:
#   sync-to-runtime.sh [--check] [stage ...]
#
#   --check   report what would change and exit 1 if anything differs; write nothing
#   stage     defaults to every stage in STAGES below
set -euo pipefail

SRC_ROOT=/workspace/infrastructure
DST_ROOT=/infra-runtime/infrastructure

STAGES=(0-persistent 1-bootstrap 2-cluster)

# Explicit allow-list per stage. A file that is not listed here is never synced,
# so a stray file in the workspace cannot reach the runtime root by accident.
managed_files() {
  case "$1" in
    0-persistent) printf '%s\n' backend.tf configuration.tf main.tf providers.tf storage.tf variables.tf ;;
    1-bootstrap) printf '%s\n' backend.tf configuration.tf controlplane.tf k3s-agents.tf main.tf providers.tf remote.tf variables.tf vpn.tf ;;
    2-cluster) printf '%s\n' backend.tf configuration.tf main.tf providers.tf variables.tf ;;
    *) echo "sync: no managed file list for stage '$1'" >&2; return 1 ;;
  esac
}

# Names that must never be written by this script, even if someone adds them to the
# allow-list by mistake. Protects Terraform state and anything holding secrets.
is_protected() {
  case "$1" in
    *.tfstate|*.tfstate.*|*.tfvars|*.tfplan|.terraform*|.config|.config/*|*.lock.info|*.pem|id_*|persistent_outputs.json) return 0 ;;
    *) return 1 ;;
  esac
}

check_only=0
args=()
for a in "$@"; do
  case "$a" in
    --check) check_only=1 ;;
    -*) echo "sync: unknown option $a" >&2; exit 2 ;;
    *) args+=("$a") ;;
  esac
done
[ ${#args[@]} -gt 0 ] && STAGES=("${args[@]}")

drift=0
for stage in "${STAGES[@]}"; do
  src="$SRC_ROOT/$stage"
  dst="$DST_ROOT/$stage"
  [ -d "$src" ] || { echo "sync: missing source stage $src" >&2; exit 1; }

  if [ "$check_only" -eq 0 ]; then
    install -d -m 0700 "$dst"
  elif [ ! -d "$dst" ]; then
    echo "would create  $dst"
    drift=1
  fi

  while read -r f; do
    if is_protected "$f"; then
      echo "sync: refusing to sync protected name '$f' in stage $stage" >&2
      exit 1
    fi
    [ -f "$src/$f" ] || { echo "sync: missing managed file $src/$f" >&2; exit 1; }

    if [ -f "$dst/$f" ] && cmp -s "$src/$f" "$dst/$f"; then
      [ "$check_only" -eq 1 ] || echo "unchanged     $stage/$f"
      continue
    fi

    drift=1
    if [ "$check_only" -eq 1 ]; then
      echo "would update  $stage/$f"
    else
      install -m 0600 "$src/$f" "$dst/$f"
      echo "updated       $stage/$f"
    fi
  done < <(managed_files "$stage")

  # Report unmanaged .tf files already in the runtime root; never remove them.
  shopt -s nullglob
  for existing in "$dst"/*.tf; do
    name=$(basename "$existing")
    if ! managed_files "$stage" | grep -qx "$name"; then
      echo "sync: NOTE unmanaged file left in place: $stage/$name" >&2
    fi
  done
  shopt -u nullglob
done

if [ "$check_only" -eq 1 ] && [ "$drift" -eq 1 ]; then
  echo "sync: runtime root differs from workspace sources" >&2
  exit 1
fi
