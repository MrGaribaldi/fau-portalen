**Title:** k3s-master: config template sets `etcd-snapshot-retention` but not `etcd-s3-retention`, so off-node snapshots keep only k3s's default of 5

**Repository:** akantodevs/infra-tools, module `shared-modules/k3s-master`

## What happens

`shared-modules/k3s-master/k3s-config.tpl.yaml` enables S3 snapshots and sets the local retention:

```yaml
etcd-s3: true
etcd-s3-folder: etcd-backups
etcd-snapshot-schedule-cron: '0 * * * *'
etcd-snapshot-retention: 168
```

It never sets `etcd-s3-retention`. On k3s releases before the fallback fix (k3s-io/k3s#13770,
backported to 1.35 in v1.35.3+k3s1), an unset `etcd-s3-retention` does not inherit
`etcd-snapshot-retention`; it uses its own default of 5. A cluster on v1.35.2+k3s1 with this
template therefore keeps 168 hourly snapshots locally, but only the last 5 in S3. So the off-node
copy, which is the one that survives losing the node, covers about 5 hours instead of 7 days.

## Evidence

Seen on a cluster built from this module (k3s v1.35.2+k3s1, hourly schedule): `k3s etcd-snapshot
list` showed 168 local snapshots but only 5 in S3. After a drop-in setting `etcd-s3-retention:
168`, S3 held hourly snapshots back to the configured week.

## Proposed change

Add the key to the template, taking the same value as the local retention so that the two cannot
drift:

```yaml
etcd-snapshot-retention: ${snapshot_retention}
etcd-s3-retention: ${snapshot_retention}
```

The module could expose `snapshot_retention` as a variable with default 168, or simply
hard-code `etcd-s3-retention: 168` next to the existing line. Declaring it explicitly is better
than relying on the upstream fallback even on newer k3s versions, because the value is then stated
rather than inherited.

## Note for existing clusters

The template feeds the module's `config_hash`, so changing it re-runs the full control-plane
install on the next apply. Users who want the fix without that can drop the key into
`/etc/rancher/k3s/config.yaml.d/` and restart k3s, which is what we did.
