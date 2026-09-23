# Stage 2, what the CCM pass left: readiness of the remaining components

Date: 10 September 2026. Owner: Infrastruktur, drift og lagringsøkonomi. Status: assessment, read
from the upstream modules rather than assumed. **Nothing here is applied and no configuration in
`infrastructure/2-cluster` has been changed** - the proposed additions are quoted in this document
so the live root keeps matching applied state and `terraform plan` keeps reporting no changes.

## First: the applied stages are clean

Checked after the CCM apply, with read-only commands only:

| Stage | `plan -detailed-exitcode` | `validate` | Resources in state |
| --- | --- | --- | --- |
| 0-persistent | 0 - no changes | valid | 12 |
| 1-bootstrap | 0 - no changes | valid | 33 |
| 2-cluster | 0 - no changes | valid | 6 |

`terraform fmt -check -recursive` on `/workspace/infrastructure` is clean. Stage 1 emits
"Values for undeclared variables" for `grafana_admin_password`, `hcloud_robot_user` and two others:
`.credentials.tfvars` is shared across stages and carries variables that stage 1 does not declare.
Cosmetic, and the alternative - a tfvars file per stage - splits the credential file that
`docs/pre-0-guide.md` deliberately keeps in one place. Left alone.

## Two blockers in the current documents are stale

Both were true when written on 9 September and are not true now. They matter because each one was
holding back a component that is in fact installable.

**cert-manager's Cloudflare API token is obsolete, not pending.** `docs/stage-2-readiness.md` says
cert-manager "needs `cloudflare_api_token`, which is empty". The 9 September certificate decision
removed the need: certificates are issued **per hostname over HTTP-01**, there is no wildcard
because FAU-er are addressed by path and never by subdomain, and that decision explicitly removes
the Cloudflare DNS-01 solver and its token from the cluster. So the token is not a missing input.
What replaces it as the real constraint is below.

**cloudnativepg's `db_memory_request` does not belong to the operator.** Upstream's wiring passes
the module nothing at all - `providers` and `depends_on` only. The memory decision on #3408 belongs
to the PostgreSQL `Cluster` resource FAU will define for the app, not to the operator install. The
operator is unblocked today.

A third is narrower than it reads: **the flux *operator* takes only `cluster_domain`.** The GitOps
repository and SOPS age wiring are needed by the `FluxInstance`/`GitRepository` configuration that
FAU has yet to author, not by the operator Helm release. This distinction is load-bearing for
#3481: the key-export trigger is secrets actually encrypted to the age key, which installing the
operator does not create.

## Component by component

| Component | Real blocker now | Installable? |
| --- | --- | --- |
| ingress-nginx | none - needs the `hcloud` provider added to FAU's root | yes, next pass |
| cert-manager | a public hostname, i.e. the domain on #3434; plus `certificate_email` | install yes, issue no |
| cloudnativepg | none | yes |
| flux (operator) | none; the GitOps repo blocks *using* it | operator yes, sync no |
| telemetry-services | Healthchecks account and two module divergences below | not yet |
| rabbitmq | no FAU use case | deliberately never |

### ingress-nginx - the recommended next pass

Upstream passes it `cluster_name` and `hcloud_location` from `persistent_outputs`,
`network_backend_subnet` and `load_balancer_name` from `bootstrap_outputs`, and `replicas = 2`. All
four values already exist from stages 0 and 1, and `lb-fau` is the load balancer it would attach to.
It needs the **`hcloud` provider**, which FAU's stage 2 root deliberately does not declare yet
(divergence 6 in `docs/stage-2-readiness.md`).

The one thing to review rather than assume: `lb-fau` already exists and is billed, created by
stage 1. Attaching ingress-nginx means the CCM starts managing that load balancer's services and
targets, so the plan will touch a live resource. That is precisely why this is a prepared pass with
a plan to read, not an apply.

Proposed addition to `infrastructure/2-cluster/providers.tf`:

```hcl
provider "hcloud" {
  token = local.persistent_outputs.hcloud_token
}
```

plus `hcloud = { source = "hetznercloud/hcloud", version = "1.66.0" }` in `required_providers`,
the exact pin the module itself requires. **One FAU divergence in the token source:** upstream's
root reads `local.bootstrap_outputs.hcloud_token`, but FAU's bootstrap outputs carry no such key -
the token lives in `persistent_outputs`, which is where the cluster module already reads it.
Copying upstream's expression would configure the provider with `null` and fail at the first API
call.

and to `main.tf`:

```hcl
module "ingress-nginx" {
  source = "${var.infra_tools_path}/shared-modules/operators/ingress-nginx${var.infra_tools_ref}"

  providers = {
    hcloud     = hcloud
    kubernetes = kubernetes
    helm       = helm
  }

  namespace              = "ingress-nginx"
  cluster_name           = local.persistent_outputs.cluster_name
  hcloud_location        = local.persistent_outputs.hcloud_location
  network_backend_subnet = local.bootstrap_outputs.network_backend_subnet
  load_balancer_name     = local.bootstrap_outputs.load_balancer_name
  replicas               = 2

  depends_on = [module.cluster]
}
```

Pre-flight checks, run live against the cluster rather than assumed:

- **The controller's `nodeSelector` is satisfied.** It requires `cloud=true` *and*
  `node-role=worker`; all three nodes carry both. Only `master` and `cworker-1` are schedulable -
  `vpn-router` keeps its `NoSchedule` taint - so `replicas = 2` places one pod on each. That does
  put an ingress controller on the control-plane node, which is a consequence of stage 1 labelling
  `master` as a worker and is worth a deliberate nod rather than a surprise.
- **Spreading is best-effort.** The chart sets `topologySpreadConstraints` with
  `whenUnsatisfiable = "ScheduleAnyway"`, not anti-affinity, so no replica can be stranded Pending
  by the spread rule.
- **It adopts `lb-fau` by annotation.** The controller Service is annotated
  `load-balancer.hetzner.cloud/name` with the load balancer name from stage 1, plus
  `use-private-ip` and `uses-proxyprotocol = true`. The CCM - installed in the last pass - is what
  acts on those annotations.

One application-level consequence to carry into #3416/#3424 rather than discover later:
`uses-proxyprotocol = true` means the real client address arrives over the PROXY protocol at the
ingress and reaches the backend as a forwarded header. ADR-001 already requires the app to trust
forwarded headers only from the configured proxy, so the two line up - but the ingress must be
configured to speak PROXY protocol on both sides or client IPs become the load balancer's.

### The placeholder hostname: fau-lab.bim.graphics

Decided 10 September 2026. `bim.graphics` is Erik's, registered at Domeneshop like the eventual FAU
domain, and it serves only a parking notice. Its Microsoft 365 records - MX, an SPF `-all` and an
`MS=` verification TXT - are leftovers from a product that was renamed years ago and are not in use.

The placeholder is a **subdomain**, never the apex, so the record is purely additive and cannot
touch the existing mail records even though they are dormant:

```
fau-lab.bim.graphics.   A      77.42.10.236
fau-lab.bim.graphics.   AAAA   2a01:4f9:c01d:3fd::1     (optional)
```

Both addresses are `lb-fau` in hel1, created by stage 1. Checked before proposing it:
`fau-lab.bim.graphics` is unused, and there is no CAA record on `bim.graphics` or on the `.graphics`
TLD, so Let's Encrypt is free to issue. Erik creates the record in the Domeneshop panel; no API
token comes into this container for a placeholder, and none is pasted into a transcript. When the
real domain on #3434 exists and its records become infrastructure-as-code, a token in the private
runtime volume earns its keep.

Two boundaries on the placeholder. It exercises the issuance path only - nothing a pilot school
sees may live under another product's domain, so #3434 still gates anything user-facing. And while
the stale M365 records are harmless, the SPF `-all` is the one record there doing useful work, since
it denies spoofing from a domain nobody is watching; dropping the MX and the `MS=` TXT is tidy,
dropping the SPF is a small downgrade.

### A staging ClusterIssuer, before production Let's Encrypt sees a placeholder

Upstream's cert-manager module defines only the **production** ACME endpoint, for both the HTTP-01
and the DNS-01 issuer. Production allows five duplicate certificates per week per hostname, and a
misconfigured challenge loop burns that in minutes - after which issuance for that name is blocked
for days. First-time HTTP-01 work belongs on staging, so FAU adds a third issuer in its own root
rather than changing the shared module.

Mirroring upstream's own technique - the issuers go in through the `raw` chart so they are created
after cert-manager's CRDs exist, which `kubernetes_manifest` cannot do at plan time:

```hcl
resource "helm_release" "fau_staging_issuer" {
  name       = "cert-manager-issuer-staging"
  chart      = "raw"
  repository = "https://charts.helm.sh/incubator"
  namespace  = "cert-manager"

  values = [yamlencode({
    resources = [{
      apiVersion = "cert-manager.io/v1"
      kind       = "ClusterIssuer"
      metadata   = { name = "letsencrypt-staging" }
      spec = {
        acme = {
          server              = "https://acme-staging-v02.api.letsencrypt.org/directory"
          email               = local.certificate_email
          privateKeySecretRef = { name = "letsencrypt-staging-key" }
          solvers             = [{ http01 = { ingress = { ingressClassName = "nginx" } } }]
        }
      }
    }]
  })]

  depends_on = [module.cert-manager]
}
```

Inherited rather than chosen: `charts.helm.sh/incubator` is a deprecated Helm repository, and
upstream's own issuer release already depends on it. FAU takes on no new supply-chain exposure by
using it here, but it is worth knowing that both issuer releases rest on an archived repo.

### The ingress-nginx pass, applied 10 September 2026

Authorized on #3485 and **applied**: 2 added, 0 changed, 0 destroyed, no errors. The plan reported
**2 to add, 0 to change, 0 to destroy**, saved as `stage2-ingress.tfplan`:
`module.ingress-nginx.kubernetes_namespace.ingress` and
`module.ingress-nginx.helm_release.nginx_ingress`. Adding the provider required
`terraform init`, which updated the runtime `.terraform.lock.hcl` with `hetznercloud/hcloud`
1.66.0.

**The plan understates what the apply does, and that is the thing to understand before applying.**
Terraform creates no `hcloud_*` resource here - it plans two Kubernetes objects. The load balancer
changes at runtime instead: the controller Service carries
`load-balancer.hetzner.cloud/name = "lb-fau"`, `location = "hel1"`,
`node-selector = "cloud=true, node-role=worker"`, `use-private-ip` and `uses-proxyprotocol`, and
the CCM acts on those to configure the load balancer. Terraform cannot show that, so a clean
two-resource plan is not evidence that nothing happens to `lb-fau`.

What lowers the risk considerably: `lb-fau` as stage 1 left it is **empty**. State shows an `lb11`
in hel1 with round-robin algorithm, no services and no targets - it has been billed since
9 September doing nothing. The CCM's work is therefore additive: it adds services and node targets
where there are none. Nor should this produce drift in stage 1, because load balancer services and
targets are separate resource types that stage 1 never declared.

**Verified after the apply.** The CCM adopted `lb-fau` as predicted: the controller Service holds
`EXTERNAL-IP 10.0.1.1, 2a01:4f9:c01d:3fd::1, 77.42.10.236` on ports 80 and 443, the `nginx`
IngressClass exists and is default, and the two controller pods run on `master` and `cworker-1`
exactly as the nodeSelector and taint analysis predicted. End to end through the public address,
nginx answers `404` over HTTP on v4 and with `Host: fau-lab.bim.graphics`, and HTTP/2 over TLS with
its own default certificate - the expected state for a cluster with no Ingress resources yet.

The DNS record was created by Erik before the apply and is live authoritatively -
`fau-lab.bim.graphics` A `77.42.10.236` and AAAA `2a01:4f9:c01d:3fd::1`, TTL 1800. Note for
anyone testing it early: `bim.graphics` has a negative TTL of 3600, so a resolver that queried the
name *before* it existed caches the miss for up to an hour. That happened in this container. It
does not affect ACME, which resolves from Let's Encrypt's side, but cert-manager's own in-cluster
self-check resolves through coredns, so a name queried too early can make the first HTTP-01 attempt
look broken when it is only cached.

**The drift prediction held.** After the apply, both stage 2 and stage 1 report `No changes`. The
services and node targets the CCM created on `lb-fau` do not appear as stage-1 drift, because load
balancer services and targets are separate resource types that stage 1 never declared. Nothing in
any namespace is outside `Running`.

Two small hygiene notes from reading the state rather than the plan: `lb-fau` has
`delete_protection = false`, which is worth turning on once it is the public entrypoint - stage 0
protects the buckets with `prevent_destroy` for the same reason. And the runtime lock file is the
only record of provider versions, since `.terraform.lock.hcl` is gitignored; that is the
established pattern here but it does mean provider pins are not reproducible from a clone.

### The cert-manager plan, prepared 10 September 2026

Authorized on #3485 and planned, **not applied**. `terraform plan`: **5 to add, 0 to change,
0 to destroy**, saved as `stage2-certmanager.tfplan`:

| Resource | Detail |
| --- | --- |
| `module.cert-manager.kubernetes_namespace.cert_manager` | namespace `cert-manager` |
| `module.cert-manager.helm_release.cert_manager` | jetstack chart, resolves to **v1.21.1** |
| `module.cert-manager.kubernetes_secret.cloudflare_api_token` | empty Secret, accepted as clutter |
| `module.cert-manager.helm_release.cert_manager_issuers` | upstream's `letsencrypt-http` and `letsencrypt-dns` |
| `helm_release.fau_staging_issuer` | FAU's `letsencrypt-staging`, `raw` chart pinned 0.2.5 |

`certificate_email` is `kontakt@ewb-solutions.as`, set as a local in `configuration.tf` beside the
placeholder hostname.

**One risk the plan exposed: the cert-manager chart version is not pinned.** Upstream's module has
`# version = "v1.15.0"` commented out, so the release takes whatever jetstack publishes as latest -
today **v1.21.1**, six minor versions beyond the number in the comment. Two consequences. An apply
months from now installs a different version than this plan describes, and a `helm_release` without
a version has no reproducibility guarantee at all. FAU cannot pin it without diverging from the
shared module, since the module exposes no version variable, so the options are: accept it and
record the installed version after each apply; ask upstream to expose a variable; or fork the
module. FAU's own staging issuer **is** pinned - `raw` 0.2.5 - which is a deliberate divergence from
upstream's unpinned issuer release.

Checked rather than assumed: `charts.helm.sh/incubator` is deprecated but still serving. Its
`index.yaml` responds, lists `raw` 0.2.5 and 0.2.4, and the 0.2.5 tarball returns HTTP 200. Both
issuer releases depend on that archived repository, so the day it stops serving, cert-manager's
issuers cannot be created or updated.

**There is no relocated home to move to.** Helm's own relocation list,
`helm/community/stable-repo-charts-new-locations.md`, covers only `stable/*` charts - 283 entries,
zero `incubator/` entries - and the chart in question is `incubator/raw`, absent from both the
"Applicable" table and the "Non-applicable (purposefully deprecated)" list. Checked 10 September
2026, so the deprecated repository stands for now and this is not worth re-checking against that
file. Third-party `raw` successors exist on Artifact Hub, but adopting an unvetted community repo
is a worse supply-chain position than a frozen repository published by the Helm project itself.

The alternative that would remove the dependency altogether: `kubectl_manifest` from the
`gavinbunney/kubectl` provider applies a CRD-based resource without the plan-time CRD problem that
made the `raw` chart necessary - and **upstream's own stage-2 root already declares that provider**
for rabbitmq. So the archived repo could be dropped using a dependency upstream already has. That
is a change for the infra-tools author rather than a FAU divergence, since it belongs in the shared
cert-manager module.

### cert-manager - installable, but it cannot issue anything yet

`certificate_email` is a one-line input from Erik. The blocker for *issuing* is that HTTP-01 needs a
public hostname that resolves to the load balancer, and the product name and domain are still open
on #3434. Installing early is possible and harmless, but it has a cost worth stating: the upstream
module creates `kubernetes_secret.cloudflare_api_token` and the `letsencrypt-dns` ClusterIssuer
**unconditionally**. With the token empty, the cluster gets an empty Secret and a second
ClusterIssuer that can never solve a challenge. Nothing references it, so it is clutter rather than
a fault - but it is exactly the kind of dead object that later reads as a misconfiguration.

The HTTP-01 issuer hardcodes `ingressClassName = "nginx"`, so ingress-nginx must be installed first
regardless. Recommendation, revised once the placeholder was decided: cert-manager can go in as soon as
ingress-nginx is up and `fau-lab.bim.graphics` resolves, issuing against **staging** first. The
production issuer waits for the real domain. The dead DNS issuer question is unchanged and still
needs a call - accept upstream's empty Secret and unusable `letsencrypt-dns`, or diverge.

### telemetry-services - the decided alerting shape mostly fits, with two real gaps

Good news first, read from `alerting.tf`: upstream's severity routing is already the shape accepted
on #3442. `critical` routes to Slack plus ntfy, `warning` to Slack plus email, and each receiver
exists only when its variable is non-empty. With Slack left empty - which #3442 decided - that
collapses to exactly **critical to the webhook, warning to email**. And `ntfy_config` is a plain
Alertmanager `webhook_configs` entry (`url`, `send_resolved`, optional bearer token from a mounted
file), not anything ntfy-specific, so the Healthchecks `/fail` endpoint can be carried by the
`ntfy_url` variable with no module change. The variable name would lie about its contents, which is
worth a comment in FAU's root rather than a fork.

Two gaps that do need resolving before this pass is planned:

1. **`send_resolved = true` is hardcoded** in `ntfy_config`. For a Healthchecks `/fail` URL that
   means a *second* failure ping when the alert resolves, which is the opposite of the intended
   semantics. Either the module gains a variable, or FAU points the webhook at a check whose
   resolve behaviour does not matter, or the receiver is configured outside this module.
2. **There is no Watchdog rule upstream.** `alerting-rules.tf` ships node-health,
   workload-health, certificate-health and log-health groups. The always-firing Watchdog that
   #3442's dead man's switch depends on - the ping whose *absence* is the alarm - does not exist
   and FAU has to author it (an `expr: vector(1)` rule with its own route to the Healthchecks
   check). Without it, the design's only mechanism for detecting cluster death is missing.

Also still true: telemetry needs its own S3 bucket (upstream creates one in `storage.tf` via the
minio provider) and `grafana_admin_password`, which already exists in `.credentials.tfvars`.

## What Erik decides before the next passes

1. ~~**Authorize the ingress-nginx pass**~~ - authorized 10 September, planned, awaiting apply
   authorization.
2. ~~**`certificate_email`**~~ - answered 10 September: `kontakt@ewb-solutions.as`.
3. **The Healthchecks account** - signup is an external action needing its own authorization, and
   the ping URL cannot be configured before it exists.
4. **Which of the two alerting gaps to accept**, since one of them - the missing Watchdog - is the
   part of #3442 that catches total cluster failure.
5. **Create `fau-lab.bim.graphics`** in the Domeneshop panel, pointing at the addresses above. Not
   blocking until ingress-nginx is applied, because HTTP-01 cannot validate before then.
