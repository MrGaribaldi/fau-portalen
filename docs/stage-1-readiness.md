# Stage 1: bootstrap network, three CX23 nodes and k3s

Date: 9 September 2026. Owner: Infrastruktur, drift og lagringsøkonomi.
Status: applied and verified. Stage 2 has not been prepared.

Stage 1 was prepared and applied on 9 September after Erik authorized the concrete plan. It was
brought forward deliberately: the cost-optimized CX category flickers in and out of stock —
Erik observed CX13 available on 8 September, nothing at all earlier on 9 September, then CX23
available — so the capacity was claimed while it existed rather than after the paperwork.

## FAU's stage-1 root

Source of truth is `/workspace/infrastructure/1-bootstrap`, synced one way into
`/infra-runtime/infrastructure/1-bootstrap` through the allow-list in `sync-to-runtime.sh`, which
now covers both stages. Terraform runs only in the runtime root. Four deliberate differences from
upstream `infra-tools/infrastructure/1-bootstrap`, all inside FAU's own root; the shared modules
are reused unchanged:

1. **All three servers are `cx23`** instead of `cpx12` + `cpx22` + `cpx22`. See
   docs/planning-decisions.md, "Node choice: start on three CX23".
2. **Upstream's `extra-firewall` resource is removed.** It opened TCP 6000-6001 to `0.0.0.0/0`
   and `::/0` on the worker to demonstrate the `firewall_ids` plumbing. FAU opens no public ports
   beyond what the modules require. The demo node label `extra-node-label=yes` went with it.
3. **State is remote**, in the same private `fau-tfstate` bucket as stage 0, under key
   `1-bootstrap/terraform.tfstate`. Upstream keeps local state in every stage. Stage 1 generates
   three more WireGuard private keys into state, so it gets the same versioned, deletion-denied
   protection stage 0 has.
4. **`infra_tools_path` is relative** (`../../../opt/infra-tools`), unlike stage 0's absolute
   `/opt/infra-tools`. This is a Terraform constraint, not a preference: the `bootstrap/*` modules
   reach their sibling shared-modules with `../` sources, and an absolute path makes Terraform
   treat that directory as a self-contained package, where a `../` source fails with "Local module
   path escapes module package". Both the workspace and runtime stage directories sit three levels
   below `/`, so one value resolves correctly in each.

## Apply

`terraform apply stage1.tfplan` from the runtime root with `umask 077`, after a reviewed plan of
31 to add, 0 to change, 0 to destroy. Result: **31 added, 0 changed, 0 destroyed**, no errors.
A fresh `terraform plan` afterwards reports "No changes" with `-detailed-exitcode` 0.

Created in Hetzner, all in `hel1`, verified read-only against the Cloud API after the fact:

| Resource | Type | Address |
| --- | --- | --- |
| `master` | cx23 | public 62.238.50.173, private 10.0.1.250 |
| `vpn-router` | cx23 | public 2.29.26.108, private 10.0.1.254 |
| `cworker-1` | cx23 | public 2.29.38.187, private 10.0.1.100 |
| `lb-fau` | lb11 | public 77.42.10.236, 0 targets so far |
| `fau-backend` | network | 10.0.0.0/14, subnet 10.0.1.0/24, 3 servers |

Plus three network routes, four firewalls, a placement group, three WireGuard keypairs, three
generated local files and twelve provisioning steps.

**Running cost starts here**: EUR 16.47 for the three servers, EUR 7.49 for the load balancer and
EUR 1.50 for three primary IPv4, about **EUR 25.46 per month net**, roughly EUR 0.84 per day.
No block volumes exist yet; those arrive with stage 2's telemetry and database charts.

## Cluster state

All three nodes joined and are `Ready` on k3s v1.35.2+k3s1, Ubuntu 24.04.4 LTS:

```
cworker-1    Ready    <none>                 10.0.1.100
master       Ready    control-plane,etcd     10.0.1.250
vpn-router   Ready    <none>                 10.0.1.254
```

**Three kube-system pods are Pending, and this is expected.** coredns,
local-path-provisioner and metrics-server cannot schedule because every node carries
`node.cloudprovider.kubernetes.io/uninitialized=true:NoSchedule`. That taint is the external
cloud-provider handshake: the Hetzner Cloud Controller Manager removes it as it initializes each
node, and the CCM is installed by stage 2. `vpn-router` additionally carries a deliberate
`node-role.kubernetes.io/vpn-router:NoSchedule` taint so general workloads stay off the router.
Nothing here needs fixing; stage 2 clears it.

## Generated files

All in `/infra-runtime/infrastructure/.config`, which is 0700:

- `bootstrap_outputs.json` — handoff to stage 2, **mode 0700 rather than 0600**. `local_file`
  wrote it executable; harmless but inconsistent with the 0600 that stage 0 applies to
  `persistent_outputs.json`, and worth tightening when stage 2 touches this directory.
- `kubeconfig.yaml` — 0600, cluster admin credentials.
- `node-token` — 0600, the k3s join token.
- `fau.conf` — 0600, the WireGuard devops client config.

`apply-20260909.log` is kept in the runtime stage directory as an audit trail. It contains the
node-token fetch, so it stays in the runtime volume and is never copied into `/workspace`.

## Reaching the cluster: the stage-2 prerequisite is now concrete

`kubeconfig.yaml` points at `https://10.0.1.250:6443`, a private address. The pre-0 guide flagged
this as an open question; it can now be stated exactly.

**WireGuard from this container does not work today.** `/dev/net/tun` does not exist and `CapEff`
is `0000000000000000` — no effective capabilities, so no `NET_ADMIN`. `wg` and `wg-quick` are
installed but cannot create an interface. Enabling it means adding `devices: /dev/net/tun` and
`cap_add: NET_ADMIN` to the agent service in `docker-compose.yml` and recreating the container
from the host, which ends the agent session and is a host-side action by the operating rules.

**The SSH bastion works today and needs no capabilities.** Every verification above was done by
jumping through the vpn-router with `ProxyCommand`, using the admin key on port 54322, and running
`k3s kubectl` on the master. For local `kubectl`, an `ssh -L 6443:10.0.1.250:6443` forward through
the same bastion is the obvious next step; k3s normally includes `127.0.0.1` in the API server
certificate SANs, which would make it work without disabling TLS verification, but that has not
been tested yet.

This is a real choice for stage 2, not a blocker discovered late: the tunnel is free and available
now, the WireGuard route is cleaner but costs a host-side container change.

> **Superseded later on 9 September 2026.** The choice stated above was decided in favour of
> WireGuard and the host-side change was made: the `agent` service now has `devices: /dev/net/tun` and
> `cap_add: NET_ADMIN`, the image carries a narrow sudo rule for `wg-quick`/`wg`, and the tunnel
> has been raised and verified from this container — `kubectl` reaches `https://10.0.1.250:6443`
> directly. The section is kept as the record of what was true when stage 1 was applied. See
> docs/planning-decisions.md and `.agents/skills/fau-vpn/SKILL.md`.

## Open items

- Stage 2 installs the CCM and CSI, which untaints the nodes and lets kube-system schedule.
- `psql-cluster` needs `db_memory_request` set in FAU's root before stage 2, or the database pods
  run BestEffort and are first to be evicted on a 4 GB node. See #3408.
- `loki` and `tempo` are unpinned upstream; pinning them is stage 2 deployment hygiene.
- `bootstrap_outputs.json` should be 0600 rather than 0700.
- The load balancer has no targets and will stay idle until there is an app to serve.
