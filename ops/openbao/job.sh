#!/bin/sh
# The deletion job's entrypoint: log in as fau-keys-operator via Kubernetes auth, then shred.
# `exec` replaces this process with `shred.sh`, so its exit status becomes the container's exit
# status unchanged — including exit 3, shred.sh's "finalize refused a young chat month" critical
# condition (docs/key-service-design.md §5). With `restartPolicy: Never` and `backoffLimit: 0`
# (ops/openbao/k8s/cronjobs.yaml), a non-zero exit here fails the Job, which is what lets
# KeyServiceShredCritical and a Kubernetes Job-failure alert fire on a refusal.
set -eu
BAO_TOKEN=$(bao write -field=token auth/kubernetes/login role=fau-keys-operator \
  jwt=@/var/run/secrets/kubernetes.io/serviceaccount/token)
export BAO_TOKEN
exec sh /ops/shred.sh "$@"
