# Stage 2 readiness: Hetzner CCM and cluster components

Prepared 9 September 2026, the same evening stage 1 was applied and accepted (#3482).
**Applied 10 September 2026** on Erik's authorization on #3483. The plan was regenerated
before the apply and came out byte-identical to the saved one - 6 to add, 0 to change,
0 to destroy - so nothing had drifted in the intervening day. Apply reported 6 added, all
three nodes lost the `uninitialized` taint, the three Pending kube-system pods went Running,
and a fresh plan afterwards reported no changes.

## Scope: the CCM pass only, not all of upstream stage 2

Upstream `infra-tools/infrastructure/2-cluster` installs eight things in one root: the
`cluster` module (CCM, CSI, coredns-custom, Reloader), ingress-nginx, cert-manager,
cloudnativepg, rabbitmq, flux and the telemetry stack. FAU's stage 2 root installs only the
`cluster` module in this pass. Each omission has a reason, not a preference:

- **cert-manager** needs `cloudflare_api_token`, which is empty in `persistent_outputs.json`.
  FAU's edge and DNS are still open, see docs/edge-dns-assessment.md.
- **flux** needs the GitOps repository and the SOPS age wiring. That is also the one thing
  that would make a key export outside Hetzner a real trigger, which is #3481.
- **telemetry-services** needs an alerting endpoint, which is the open question on #3442,
  and a `db_memory_request` decision travels with cloudnativepg (#3408).
- **rabbitmq** has no FAU use case in the MVP scope.
- **ingress-nginx** is wanted, but it belongs with cert-manager and the DNS decision rather
  than ahead of them.

Installing the CCM alone is what clears the `node.cloudprovider.kubernetes.io/uninitialized`
taint and lets the three Pending kube-system pods schedule. That is the immediate goal and
it stands on its own.

## What the plan does

`terraform plan` reported **6 to add, 0 to change, 0 to destroy**, saved to `stage2.tfplan`
in the runtime stage directory. That plan file has been applied and removed; a spent plan
file left in the directory is only a stale-apply hazard.

| Resource | Detail |
| --- | --- |
| `kubernetes_secret.hcloud_token` | Secret `hcloud` in `kube-system`: API token, network id, empty robot user/password |
| `helm_release.hcloud_ccm` | `hcloud-cloud-controller-manager` 1.31.1 from charts.hetzner.cloud, `kube-system` |
| `helm_release.hcloud_csi` | `hcloud-csi` 2.16.0, pinned to the GitHub release tarball, `kube-system` |
| `kubernetes_config_map.coredns_custom` | `coredns-custom` in `kube-system`, no forward rules |
| `kubernetes_namespace.reloader` | `reloader-system` |
| `helm_release.reloader` | Stakater Reloader 2.2.16, `reloader-system` |

Every `helm_release` waits for readiness with a 300-second timeout.

## FAU divergences from upstream, all inside FAU's own root

1. Only the `cluster` module, as above.
2. `forward_dns_zones` is empty. Upstream forwards `cluster.telemetry.local` to
   `10.30.0.10`, a separate telemetry cluster FAU does not have. The `coredns-custom`
   ConfigMap is still created, with an empty `forward.server` key.
3. No alerting locals. Upstream's `configuration.tf` hardcodes a Slack channel, a public
   `ntfy.sh` topic and an SMTP2GO smarthost. All three were dropped as upstream examples
   (#3442) and the replacement is undecided, so none is carried over.
4. `certificate_email` is not set; upstream's belongs to cert-manager.
5. State is remote in `fau-tfstate` under `2-cluster/terraform.tfstate`, like stage 1 and
   unlike upstream's local state. This stage's state holds the hcloud API token, because
   the token is written into the cluster as a Secret.
6. Only the `kubernetes` and `helm` providers are declared. Upstream also declares hcloud,
   kubectl and minio, which belong to modules this pass does not install.

## Pre-flight checks, run before the plan

- **Node labels resolve.** The CSI and Reloader selectors need `cloud=true`; all three nodes
  carry it. The CCM selector needs `node-role.kubernetes.io/control-plane=true`; `master`
  carries it.
- **The CCM can schedule on a tainted node.** Chart 1.31.1's Deployment template tolerates
  `node.cloudprovider.kubernetes.io/uninitialized=true:NoSchedule` by default, so the
  300-second wait will not deadlock on the taint it exists to remove. Checked by reading the
  chart, not assumed.
- **The empty ConfigMap value is accepted.** `forward.server` renders as an empty string
  when `forward_dns_zones` is empty; a server-side dry-run of that ConfigMap was accepted by
  the API and persisted nothing.
- **The plan reached the cluster** at `https://10.0.1.250:6443` through the WireGuard
  tunnel, with no bastion and no port-forward.

## Known and expected

- **Corrected by the apply:** this document predicted the CSI node DaemonSet would not
  schedule on `vpn-router` because of its deliberate
  `node-role.kubernetes.io/vpn-router:NoSchedule` taint. It does schedule there - 3 of 3
  nodes ready. The chart gives `hcloud-csi-node` blanket `operator: Exists` tolerations for
  both `NoSchedule` and `NoExecute`, so a keyed taint does not keep it off. The prediction
  came from reading the taint and not the DaemonSet's tolerations. Nothing to fix: a CSI
  node plugin on every node is the intended shape, and no other workload gains access to
  `vpn-router` from it.
- `bootstrap_outputs.json` was 0700 rather than 0600; tightened to 0600 during this pass.

## How it was applied, and the rule for the next saved plan

A saved plan is only valid against the configuration and cluster state it was generated
from. If either has changed - a `2-cluster` source edit, a re-sync to the runtime root, a
new state serial, or anything altered on the cluster out of band - regenerate the plan and
read it again rather than applying the saved file. Terraform will refuse a plan whose state
has moved on, but it cannot detect a cluster changed underneath it. This pass regenerated
rather than trusting the day-old file, and diffed the new plan text against the old before
applying.

The WireGuard tunnel must be up first, or nothing can reach the API server.

```
sudo wg show fau || sudo wg-quick up /infra-runtime/infrastructure/.config/fau.conf
export KUBECONFIG=/infra-runtime/infrastructure/.config/kubeconfig.yaml
cd /infra-runtime/infrastructure/2-cluster && umask 077
terraform plan -input=false -out=stage2.tfplan   # read it before applying
terraform apply stage2.tfplan
```

Afterwards the three Pending kube-system pods should schedule, and `kubectl get nodes -o json`
should show no `uninitialized` taint.

Source of the configuration is `/workspace/infrastructure/2-cluster`, synced to the runtime
root with `infrastructure/sync-to-runtime.sh 2-cluster`.
