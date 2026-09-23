**Title:** `k3s-master` module's `backup-k3s-server.sh` calls `aws s3 cp` but the module never gives
the AWS CLI credentials, so any replace of `null_resource.k3s_master_install` deadlocks

**Type:** defect. Latent from the day a cluster is built; fires the first time the resource is
replaced, which is typically during a credential rotation or a configuration change.

## Where

`shared-modules/k3s-master/main.tf`.

`null_resource.k3s_master_install` has a destroy-time provisioner:

```hcl
provisioner "remote-exec" {
  when = destroy
  inline = [
    "echo '*** Taking final etcd snapshot before destroy ***'",
    "/root/backup-k3s-server.sh",
    "systemctl stop k3s || true",
    "echo '*** Done ***'",
  ]
}
```

and `backup-k3s-server.sh`, rendered and uploaded by the same resource, begins with `set -e` and
runs:

```bash
export AWS_ENDPOINT_URL="https://hel1.your-objectstorage.com"
aws s3 cp /tmp/$ARCHIVE_NAME $S3_DIR/$ARCHIVE_NAME
```

## The defect

The module uploads exactly three files: `k3s_config.yaml`, `backup-k3s-server.sh` and
`install_k3s_server.sh`. **None of them configures the AWS CLI.** The S3 credentials it does place
on the node go into `/etc/systemd/system/k3s.service.d/s3.conf`, which is an `EnvironmentFile` for
the k3s *service* and is not visible to a root shell, so it never reaches the `aws` process started
by the backup script.

Verified on a running FAU master, 22 September 2026:

```
$ aws s3 ls s3://fau-k3s-backup/
aws: [ERROR]: An error occurred (NoCredentials): Unable to locate credentials.
exit=253
```

`NoCredentials`, not `InvalidAccessKeyId` — the CLI has never had credentials, with any key pair.

## Why it deadlocks rather than merely failing

Because `set -e` makes the script exit non-zero, the destroy provisioner fails at the upload,
*before* `systemctl stop k3s`. Terraform cannot complete the replace. The node's configuration is
only rewritten by the **create** provisioner, which cannot run until the destroy provisioner
succeeds. So the resource can never be replaced, and the failure mode is a stuck apply rather than
a missing backup.

The trap is invisible until something forces a replace. In FAU's case that was rotating the S3
credentials after they leaked, which changes `config_hash` and therefore replaces the resource.

## Impact

1. **No server archive has ever been uploaded** to `s3://<bucket>/server-backups` on any cluster
   built with this module. The etcd snapshots in `etcd-backups/` are unaffected: k3s uploads those
   itself, using the systemd `EnvironmentFile`, which does work.
2. **Any replace of `k3s_master_install` fails**, including legitimate configuration changes.

## Fix, in preference order

1. Have the module render and upload an AWS CLI credentials file - `/root/.aws/credentials` plus a
   `/root/.aws/config` carrying region and endpoint - from the same variables it already receives.
   This makes the backup script work as written and removes the deadlock.
2. Or render the credentials into the backup script's own environment, so no long-lived file is
   added to the node.
3. Or make the destroy provisioner tolerant: `/root/backup-k3s-server.sh || true`, so a failed
   final backup cannot block a replace. This is worth doing regardless of 1 or 2, because a
   destroy-time backup that can block a destroy is the wrong trade.

## Workaround applied in FAU, 22 September 2026

`/root/.aws/credentials` and `/root/.aws/config` were written on `master` by hand, mode 0600, from
the same values in `persistent_outputs.json`. The replace then completed normally and the
destroy-time backup ran to completion for the first time. The files are outside Terraform's
management, so they are recorded in CLAUDE.md's list of places secrets live on the nodes, and they
must be rewritten on any future credential rotation until the module is fixed.
