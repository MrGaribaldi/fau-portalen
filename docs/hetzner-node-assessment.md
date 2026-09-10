# Hetzner cloud nodes for the FAU MVP

Checked 7 September 2026. Recommendation only; no infrastructure configuration changed and no servers ordered. Follow the agreed three-server infra-tools topology: VPN router, one control-plane server also labelled as worker, and one worker. Keep existing services and load balancer.

## Recommendation

The existing CPX12 + CPX22 + CPX22 is a reasonable low-cost deployment experiment, but its two workload nodes only have 4 GB each. For a less constrained starting point with the full telemetry stack retained, recommend CPX12 + CPX32 + CPX32 (8 GB per workload node), subject to Erik's selection and measured validation. This is an engineering estimate, not evidence that 4 GB fails or 8 GB is sufficient for a known number of customers.

Do not size only for Rust: the Kubernetes platform, two PostgreSQL instances, Grafana, Loki, Tempo, VictoriaMetrics, RabbitMQ and operators are the main initial sizing uncertainty. Direct S3 uploads reduce application bandwidth, but do not eliminate validation work, database activity or monitoring costs.

## Regular Performance candidates

Germany/Finland new-order monthly caps, EUR excluding VAT and Primary IPv4. Specifications from the official Regular Performance catalogue; prices from the June 2026 price schedule and website's public price data retrieved today.

| Type | Shared vCPU | RAM | Local NVMe | Monthly EUR | Assessment |
|---|---:|---:|---:|---:|---|
| CPX12 | 1 | 2 GB | 40 GB | 11.49 | Appropriate router candidate; not recommended for full app/database/telemetry node. |
| CPX22 | 2 | 4 GB | 80 GB | 19.49 | Existing master/worker default; smallest candidate to test, limited headroom. |
| CPX32 | 4 | 8 GB | 160 GB | 35.49 | Recommended master/worker starting candidate with full existing stack. |
| CPX42 | 8 | 16 GB | 320 GB | 69.49 | Growth option if actual memory/CPU measurements justify it. |
| CPX52 | 12 | 24 GB | 480 GB | 100.49 | No demonstrated MVP need. |
| CPX62 | 16 | 32 GB | 640 GB | 129.99 | No demonstrated MVP need. |

CPX11 is listed in US locations on the current catalogue; CPX12 is the small EU plan and was introduced there in July 2026. CPX is shared CPU: additional vCPUs are not a promise of continuously dedicated cores.

## Three-server costs

| Option | VPN | Master + workloads | Worker | Servers only | Servers + 3 IPv4 + LB11 + S3 base |
|---|---|---|---|---:|---:|
| Existing configuration | CPX12 | CPX22 | CPX22 | EUR 50.47 | EUR 65.95 |
| Intermediate candidate | CPX12 | CPX32 | CPX22 | EUR 66.47 | EUR 81.95 |
| Recommended candidate | CPX12 | CPX32 | CPX32 | EUR 82.47 | EUR 97.95 |

The intermediate option leaves the 4 GB node as a constraint; workloads need scheduling that respects the uneven capacity. Recommendation does not change the topology or remove existing services.

Ancillary calculation: three Primary IPv4 at EUR 0.50 each, existing LB11 EUR 7.49, Object Storage base EUR 6.49 per account. S3 base is counted once, not per bucket. These are fixed-cost subtotals, not complete invoices. Add provisioned block volumes, excess storage/traffic, optional server snapshots/backups, email and domain. No NOK exchange rate assumed. A paid FAU contributes NOK 2,400/year excluding VAT; break-even cannot be calculated honestly without total costs and an exchange rate.

Explicit volume declarations already total 250 Gi before any chart-default PVCs: PostgreSQL 2 x 10 Gi, VictoriaMetrics 2 x 100 Gi, Grafana 10 Gi, RabbitMQ 10 Gi and Alertmanager 10 Gi. Final volume billing depends on rendered PVCs, provisioner rounding and current rate; no exact volume total is claimed here. Larger local server disks do not replace these network-volume allocations automatically.

## Other families

CX cost-optimized x86 is worth knowing about: CX23/CX33/CX43/CX53 list at EUR 5.49/8.49/15.99/29.49 excluding IPv4 and VAT. CX33's 8 GB would be attractive on price. However, Hetzner's cloud page currently labels the cost-optimized category unavailable, and its hardware/performance differs from CPX. Do not base the Friday plan on unconfirmed availability. This is an alternative to discuss, not a selected change.

CAX is ARM and would require validating every image and build/deployment component; no ARM compatibility audit was done. CCX provides dedicated CPU, useful if sustained CPU contention becomes a measured issue; it is not justified by the present unmeasured MVP demand. Keeping CPX x86 best matches the existing configuration.

## Evidence in infra-tools

- infrastructure/1-bootstrap/configuration.tf: cpx12 VPN, cpx22 master/worker, lb11; master labelled node-role=worker.
- infrastructure/1-bootstrap/controlplane.tf and k3s-agents.tf: single control-plane server; VPN agent tainted NoSchedule. Three servers therefore do not mean three interchangeable workload nodes or three control-plane replicas.
- shared-modules/psql-cluster/variables.tf: 2 database instances, 10Gi each; CPU/memory requests unset by default. main.tf: required pod anti-affinity across worker-labelled nodes.
- infrastructure/2-cluster/main.tf: full telemetry and operators enabled.
- shared-modules/telemetry-services/grafana.tf: 1Gi memory request, 2Gi limit; 10Gi volume.
- shared-modules/telemetry-services/variables.tf: 2 vmstorage instances at 100Gi each; 1 vminsert and 1 vmselect; tempo_replicas input 2. Render Helm charts before treating values as actual pod counts/resources, especially where chart versions are unpinned.
- shared-modules/telemetry-services/alerting.tf: Alertmanager 10Gi PVC; explicit alerting memory requests.
- shared-modules/rabbitmq-cluster: one replica by default, 10Gi volume and 256Mi request.

The configuration does not provide a complete memory reservation budget; Helm defaults and actual workload matter. Grafana alone reserves 1 Gi, so 4 GB nodes should not be described as comfortably sufficient without measurement. Preserving the author's architecture does not prove capacity for our application.

## Validation before declaring a size suitable

Render the exact charts; inventory pods, requests and PVCs. On an authorized test deployment measure per-node memory, CPU contention, database latency, autosave latency and disk use while idle, under representative concurrent edits/uploads and during backup. Record workload parameters rather than inventing a supported FAU count. Check rescheduling and restore behavior. Suggested acceptance target for review: at least 25% memory headroom at representative peak, no OOM/eviction/pending critical pods, acceptable measured latency and successful backup/restore. These thresholds are proposals, not agreed requirements.

One control-plane server remains a control-plane failure point even with two database replicas. A larger node does not change that property. No three-control-plane redesign is proposed.

Live orderability for the selected location was not verified through an authenticated Cloud API/Console. Static page text contains availability placeholders and cannot certify inventory. Check immediately before ordering; no capacity is reserved by this research.

## Primary sources and reproducibility

- Specs/family: https://www.hetzner.com/cloud/regular-performance/
- Current new-order price schedule: https://docs.hetzner.com/general/infrastructure-and-availability/price-adjustment/
- CPX12 EU introduction: https://docs.hetzner.cloud/whats-new
- IPv4/shared resources: https://docs.hetzner.com/cloud/servers/overview/
- Cost-optimized alternative: https://www.hetzner.com/cloud/cost-optimized/ and https://www.hetzner.com/cloud/
- Load balancer: https://www.hetzner.com/cloud/load-balancer/
- Object Storage: https://www.hetzner.com/storage/object-storage/ and https://docs.hetzner.com/storage/object-storage/overview/
- Public website price data: https://www.hetzner.com/_resources/app/data/app/live_data_prices.json
- Website API for S3 base: https://website-price-api.hetzner.com/api/v1/products/CLOUD_84

Public website component product IDs: CPX12 CLOUD_122, CPX22 CLOUD_124, CPX32 CLOUD_126, CPX42 CLOUD_128, CPX52 CLOUD_130, CPX62 CLOUD_131; Primary IPv4 CLOUD_21; LB11 CLOUD_25; S3 base CLOUD_84. Selected price evidence is saved alongside this report. The price API is the website's public loader, not a deployment API. Prices do not establish orderability.

## Orderability and prices verified against the authenticated Cloud API — 9 September 2026

This closes the "live orderability" gap recorded above, which the static price pages could not
certify. Read-only `GET` calls against `api.hetzner.cloud/v1` with FAU's own Cloud token, at
2026-09-09T15:06Z. No resource was created, changed or reserved.

### Orderability: all candidates available in all three EU locations

| Datacenter | City | CPX12 | CPX22 | CPX32 | CPX42 |
| --- | --- | --- | --- | --- | --- |
| `fsn1-dc14` | Falkenstein, DE | available | available | available | available |
| `hel1-dc2` | Helsinki, FI | available | available | available | available |
| `nbg1-dc3` | Nuremberg, DE | available | available | available | available |

Every candidate type is orderable in `hel1`, where the stage-0 buckets already live, so the
recommended CPX12 + 2×CPX32 can be ordered without a location compromise. Availability is a
point-in-time reading and reserves nothing — re-check immediately before ordering.

### Prices confirmed, and the missing volume rate found

Every figure in the table above is confirmed by the API, identical across `fsn1`, `hel1` and
`nbg1`, net and excluding VAT: CPX12 11.49, CPX22 19.49, CPX32 35.49, CPX42 69.49. Ancillaries
confirmed too: Primary IPv4 0.50, LB11 7.49. The API reports `vat_rate 0.0` for this account.

Two additions the earlier research did not have:

- **Block volume rate: EUR 0.0572 per GB per month, net.** The 250 Gi of explicitly declared
  volumes therefore costs roughly **EUR 14.30–15.35 per month**, the range being whether the
  provisioner bills 250 GB or the true 268.4 GB. This is the previously unpriced line item.
- **20 TB of traffic is included per server** in all three EU locations, which removes bandwidth
  from the list of cost risks at MVP scale.

So the recommended candidate is EUR 97.95 in fixed monthly cost plus roughly EUR 15 of declared
volumes, before chart-default PVCs, snapshots, email and domain.

### Why an exact PVC total is still not claimable

Two independent obstacles, neither of which is a missing measurement:

1. **`helm` is not installed in the agent container**, so the charts cannot be rendered here to
   enumerate the PVCs the Helm defaults create on top of the declared ones.
2. **Two of the five telemetry charts are unpinned upstream.** `loki.tf` and `tempo.tf` carry
   commented-out version constraints (`# version = ">=6.0.0"`), while grafana 10.5.15,
   victoria-metrics-cluster 0.42.0 and victoria-metrics-alert 0.47.0 are pinned. A render against
   unpinned charts is not reproducible, so the number it produced would not stay true.

Pinning has to come before rendering, which makes exact PVC costing a decision rather than a task:
pin `loki` and `tempo` and add `helm` to `Dockerfile.agent` (a host-side rebuild), or accept the
bounded estimate above for the MVP decision. Actual resource measurement remains blocked on a
running cluster either way.

### Cost-optimized CX is now orderable — 9 September 2026

This supersedes the note above that Hetzner's page labelled the cost-optimized category
unavailable. Verified against the authenticated API, same read-only session:

| Type | Arch | vCPU | RAM | Local disk | EUR/mo net (hel1) | Orderable in fsn1 / hel1 / nbg1 |
| --- | --- | --- | --- | --- | --- | --- |
| CX23 | x86 | 2 | 4 GB | 40 GB | 5.49 | **yes, all three** |
| CX33 | x86 | 4 | 8 GB | 80 GB | 8.49 | priced and supported, **out of stock in all three** |
| CX43 | x86 | 8 | 16 GB | 160 GB | 15.99 | priced and supported, **out of stock in all three** |
| CAX21 | **arm** | 4 | 8 GB | 80 GB | 10.49 | yes, all three |
| CAX31 | **arm** | 8 | 16 GB | 160 GB | 20.99 | yes, all three |

CX23 includes the same 20 TB of traffic as CPX in all three EU locations.

The comparison that matters is CX23 against CPX22: identical 2 vCPU and 4 GB, half the local disk
(40 against 80 GB), at 5.49 against 19.49. There is no reason to pay CPX22 prices for CX23
specifications, so the existing CPX12 + 2×CPX22 configuration is now the wrong shape at the wrong
price.

Three-node options, per month net, excluding the EUR 15.48 of ancillaries (3 × IPv4, LB11, S3
base) and roughly EUR 15 of declared volumes:

| Configuration | Total RAM | Disk per node | EUR/mo |
| --- | --- | --- | --- |
| 3 × CX23 | 12 GB | 40 GB | 16.47 |
| 3 × CX33 | 24 GB | 80 GB | 25.47 — not orderable today |
| 3 × CAX21 (ARM) | 24 GB | 80 GB | 31.47 |
| CPX12 + 2 × CPX22 (existing plan) | 10 GB | 40 / 80 GB | 50.47 |
| CPX12 + 2 × CPX32 (recommended above) | 18 GB | 40 / 160 GB | 82.47 |

Three identical CX23 give more total memory than the existing plan for a third of the price, and
remove the asymmetry where the VPN and master node has only 2 GB. What they do not do is answer
the concern that produced the CPX32 recommendation: 4 GB per node with the full telemetry stack
retained. After k3s and system overhead that leaves roughly 3 to 3.5 GB usable per node, and
Grafana alone requests 1 Gi.

Two facts make that risk cheap to take rather than something to price around. A Hetzner server
type change is an in-place rescale plus a reboot, not a rebuild, so CX23 to CX33 or CX43 is a
later resize once stock returns — and since PostgreSQL and telemetry data sit on `hcloud-volumes`
CSI network volumes rather than local disk, the 40 GB local disk carries images, logs and
ephemeral data only. Rescaling upward is allowed; rescaling to a *smaller* disk is not, so
starting small keeps the path open in the direction we would actually need.

The one lock-in to be deliberate about: rescaling cannot cross architectures. Choosing CX23 keeps
every x86 upgrade open. Choosing CAX21 would buy 8 GB per node for EUR 10.49 — cheaper than the
existing plan with more memory than the recommended one — but requires the ARM audit this
assessment has never done: arm64 images for the Rust build, the Node frontend build, and every
chart image including Grafana, Loki, Tempo, VictoriaMetrics, the RabbitMQ operator and CNPG.
Most publish arm64 today; "most" is not "verified".

### Decision: start on 3 × CX23 — 9 September 2026

Erik chose three CX23 as the starting point, with CX33 as a later order if needed. This section
records the fit assessment behind that, from declared resources in the modules — not from
measurement, which still needs a running cluster.

**Total cost.** 3 × CX23 at 5.49 = EUR 16.47, plus EUR 15.48 of ancillaries (3 × IPv4, LB11, S3
base) and roughly EUR 15 of declared volumes: about **EUR 47 per month net**, against about
EUR 113 for the CPX12 + 2 × CPX32 recommendation. The 250 Gi of volumes does not change with node
size, so the saving is entirely in compute.

**Will it fit?** Scheduling admission is not the constraint. Explicit memory requests across every
module total roughly 1.85 GB:

| Component | Memory request | Limit |
| --- | --- | --- |
| Grafana | 1 Gi | 2 Gi |
| ingress-nginx | 256 Mi | 1 Gi |
| RabbitMQ | 256 Mi | unset |
| vmalert | 128 Mi | 512 Mi |
| Alertmanager | 64 Mi | 256 Mi |
| kube-state-metrics | 100 Mi | 300 Mi |
| grafana-gitsync | 32 Mi | 128 Mi |
| external-metrics-collector | 32 Mi | 128 Mi |

Three 4 GB nodes give roughly 9 GB usable for pods, after the k3s server on the master and agents
elsewhere. So declared requests leave around 7 GB of headroom and everything schedules.

**What the numbers do not cover.** Loki, Tempo, VictoriaMetrics and Alloy declare no resources at
all, so they inherit chart defaults, and `loki.tf` and `tempo.tf` are unpinned — the same reason
an exact PVC total is not claimable. Their real consumption grows with ingestion rather than with
requests. VictoriaMetrics runs `vmstorage` at 2 replicas with `vminsert` and `vmselect` at 1 each.
Grafana alone requests a quarter and may use half of one node.

**The catch worth fixing before stage 2.** `psql-cluster` leaves `db_memory_request` null by
default, and upstream's own comment explains the consequence: the instance pods "run BestEffort
and are the first thing evicted under node memory pressure". On 4 GB nodes that makes the database
the first casualty of any memory spike, which is precisely backwards. FAU's root must set
`db_memory_request` — 512Mi is a sensible starting value — to get Burstable QoS. Leave
`db_memory_limit` null, as upstream advises, because a memory limit turns pressure into an
OOMKill of the primary. The default is null deliberately: the module is shared between stage and
prod roots, so opting in happens per root.

**The exit if 4 GB proves too tight.** A type change is an in-place rescale plus a reboot, not a
rebuild: CX23 to CX33 is x86 to x86 and grows the disk 40 to 80 GB, which is the permitted
direction. PostgreSQL and telemetry data live on `hcloud-volumes` network volumes, so nothing
migrates. CX33 is priced and supported but out of stock in all three EU datacenters today, so the
upgrade waits on Hetzner inventory rather than on us.

**Exact PVC costing: not pursued.** Erik accepted the EUR 14.30–15.35 volume estimate on
9 September rather than pin `loki` and `tempo` and add `helm` to the agent image for a precise
figure. The estimate covers the 250 Gi of explicitly declared volumes at the API-verified rate of
EUR 0.0572 per GB per month; chart-default PVCs on top of it remain unquantified, and the first
real invoice will settle it more cheaply than a render would have. Pinning the two unpinned charts
is still worth doing for reproducibility, but it is deployment hygiene rather than a costing task,
and belongs with the stage 2 configuration work.
