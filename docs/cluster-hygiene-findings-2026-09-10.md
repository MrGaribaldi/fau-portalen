# Cluster hygiene findings after the CCM and ingress passes

Date: 10 September 2026. Owner: Infrastruktur, drift og lagringsøkonomi. Read-only inspection of
the applied cluster; nothing was changed. Both findings need a decision rather than a fix from me.

## 1. Off-node etcd retention is about 5 hours, not the configured 7 days

Backups do work, and my first reading of this was wrong: I saw only the first page of
`kubectl get etcdsnapshotfile`, found nothing but `file://` locations, and concluded the private
`fau-k3s-backup` bucket was empty. It is not. The k3s journal shows hourly `S3 upload complete`
lines and the newest S3 snapshot is `ready=true`, 6.4 MB, uploaded at 11:00Z.

The real finding is retention, and it is a per-location asymmetry from a single setting. The
deployed `/etc/rancher/k3s/config.yaml` on `master` says `etcd-snapshot-retention: 168` with
`etcd-snapshot-schedule-cron: '0 * * * *'` - hourly snapshots kept for a week. Actual counts, by
location and snapshot-name prefix:

| Location | `k3s-master` (scheduled, hourly) | `backup-…` (one-off, from the install script) |
| --- | --- | --- |
| local (`/var/lib/rancher/k3s/server/db/snapshots`) | **15** - every snapshot since the cluster came up | 1 |
| S3 (`s3://fau-k3s-backup/etcd-backups`) | **5** | 1 |

Five is k3s's *default* retention, so the S3 pruning path appears not to honour the configured
value on k3s v1.35.2+k3s1, while the local path does. The journal confirms active pruning: every
hour, one `S3 upload complete` is followed by `Removing S3 snapshot: s3://fau-k3s-backup/…` of the
snapshot from roughly six hours earlier. Cause not established - this is an observation with a
signature, not a diagnosis.

**Why it matters.** The off-node copy exists because `master` can be lost. With five hourly
snapshots there, a problem discovered more than about five hours after it started has no off-node
snapshot from before it: the only older off-node copy is the one-off `backup-…` snapshot taken at
install time on 9 September. The week of history that the configuration promises exists only on
`master` - the node whose failure the bucket is there to survive.

Also seen once in eight hours, and worth watching rather than acting on:
`Error retrieving S3 snapshots for reconciliation: context deadline exceeded`. A single transient
timeout listing the bucket. If it becomes frequent, retention reconciliation is the thing it would
affect.

### Cause established, 10 September 2026: a second flag we never set

Erik chose option (a) - check upstream first - and it resolved the question without a code change
being needed anywhere clever. **S3 retention is a separate setting with its own default of 5.**
From the binary on our own node:

```
--etcd-snapshot-retention value   (db) Number of snapshots to retain (default: 5)
--etcd-s3-retention value         (db) S3 retention limit (default: 5)
```

FAU's config sets `etcd-snapshot-retention: 168` and never sets `etcd-s3-retention`, so S3 keeps
the default 5 while local keeps 168. The observed asymmetry is exactly the configuration, not a
defect.

The upstream history explains why it looks like a bug. k3s added the dedicated S3 flag
(k3s-io/k3s#12669, #12671), then added a fallback so an unset `etcd-s3-retention` inherits
`etcd-snapshot-retention` (#13770), and the release-1.35 backport of the related regression
(#13783) carries milestone **v1.35.3+k3s1**. The cluster runs **v1.35.2+k3s1** - one patch short of
the version where leaving the flag unset would have done the right thing. Current 1.35 line is at
v1.35.8+k3s1.

**Recommended fix: set `etcd-s3-retention: 168` explicitly.** It works on the version we already
run, needs no upgrade, and is better than relying on the fallback even after an upgrade, because
the value is then declared rather than inherited. Cost is negligible: 168 snapshots at the current
~6.4 MB is about 1.1 GB, a fraction of a cent per month at Hetzner's object storage pricing.

Where to put it, in order of blast radius:

1. **A FAU-side drop-in.** k3s reads `/etc/rancher/k3s/config.yaml.d/*.yaml` in addition to
   `config.yaml`; that directory does not exist on `master` today. A small `null_resource` in FAU's
   own `1-bootstrap` root can write `10-fau-s3-retention.yaml` and restart k3s - in code, as Erik
   asked, and it touches one file. **It does restart the API server on the single control-plane
   node**, so it needs authorization and a moment when a brief outage is acceptable.
2. **Ask upstream to add the flag** to `k3s-master/k3s-config.tpl.yaml`. Right long-term home, and
   it would let the FAU drop-in retire - but applying it changes `config_hash`, which re-triggers
   `null_resource.k3s_master_install` and re-runs the whole control-plane install script. Much more
   invasive than the drop-in, so this is the follow-up, not the fix.

Option (c) - scheduling the module's `backup-k3s-server.sh` - is no longer needed for retention. It
remains useful for a different reason: it also uploads a full server-directory archive, which an
etcd snapshot alone does not cover. Option (b) is unnecessary once retention is correct, and both
buckets have `object_locking = false`, so a bucket-side guard would have needed enabling that
first. Restore testing still belongs to #3425; retention only decides what a restore can work
from.

## 2. Two default StorageClasses, and the CCM pass is what created the ambiguity

```
NAME                       PROVISIONER             DEFAULT
hcloud-volumes             csi.hetzner.cloud       yes   (166m - installed by today's CCM pass)
local-path                 rancher.io/local-path   yes   (16h  - k3s packaged addon)
```

Both carry `storageclass.kubernetes.io/is-default-class: "true"`. `hcloud-volumes` gets it from the
`hcloud-csi` Helm release, which **I installed this morning**; `local-path` gets it from k3s's
packaged `local-storage` Addon, visible in its `objectset.rio.cattle.io/owner-name` annotation. So
this is a consequence of the CCM pass that the pre-flight checks in
docs/stage-2-remaining-readiness.md did not anticipate.

**Why it matters.** Which class a PVC without an explicit `storageClassName` receives is
version-dependent when several are default, and not something to rely on. For FAU the bad outcome
is specific: a PostgreSQL volume landing on `local-path` is node-local storage that dies with the
node, on a database whose whole point is audited history that survives.

**Options.** (a) **Recommended, and free:** make it a rule that every PVC and every stateful
workload sets `storageClassName` explicitly - cloudnativepg's `Cluster` takes one - so the default
never decides anything that matters. (b) Remove the annotation from `local-path`: correct in
principle, but k3s's addon controller owns that object and will reassert it, so it is not durable.
(c) Add `local-storage` to the `disable:` list in the k3s config, which is the durable fix and the
expensive one: that list lives in upstream's `k3s-master/k3s-config.tpl.yaml`, the file is part of
`config_hash`, and changing it re-triggers `null_resource.k3s_master_install` - a control-plane
re-provision. Not worth doing on its own; worth folding in the next time stage 1's control-plane
config is touched for another reason.

## Method note

Everything above is read-only: `kubectl get`, `terraform state show`, and two `ssh` commands to
`master` on port 54322 that read the journal and the config file with credential lines filtered out.
No secrets were printed and nothing was modified.
