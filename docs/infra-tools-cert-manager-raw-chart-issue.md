**Title:** cert-manager module depends on the archived `charts.helm.sh/incubator` `raw` chart, and this repo already uses the idiom that replaces it

**Type:** maintenance / dependency removal. Nothing is broken today; this is about a dependency
that cannot be updated and has no relocated home.

## Where

`shared-modules/operators/cert-manager/cluster-issuer.tf` installs the two ClusterIssuers through
`helm_release.cert_manager_issuers`:

```hcl
chart      = "raw"
repository = "https://charts.helm.sh/incubator"
```

`charts.helm.sh/incubator` is the archived Helm incubator repository, deprecated with `stable` in
November 2020. It receives no updates.

## What was checked, 10 September 2026

- **Still serving.** `index.yaml` responds, lists `raw` 0.2.5 and 0.2.4, and the 0.2.5 tarball
  returns HTTP 200. So this is not urgent.
- **No relocated home exists.** Helm's own relocation list,
  `helm/community/stable-repo-charts-new-locations.md`, covers `stable/*` charts only - 283 entries,
  zero `incubator/` entries - and `incubator/raw` appears in neither its "Applicable" table nor its
  "Non-applicable (purposefully deprecated)" list. There is nothing to point an upgrade at.
- **Third-party successors exist** on Artifact Hub, but adopting an unvetted community repo is a
  worse supply-chain position than a frozen repository published by the Helm project itself.
- The release is also **unpinned** - no `version` - so it resolves to whatever the frozen index
  offers, today 0.2.5.

## Proposal: `kubectl_manifest`, which this repo already uses

The `raw` chart exists here to work around a real constraint, stated in the file's own comment: a
ClusterIssuer is a cert-manager CRD, and `kubernetes_manifest` cannot plan a resource whose CRD does
not yet exist, so the issuers must be created after cert-manager is deployed.

`kubectl_manifest` from `gavinbunney/kubectl` does not evaluate the schema at plan time, so
`depends_on` is sufficient ordering and the chart is unnecessary. Two things make this cheap rather
than a new dependency:

- The provider is **already declared** in `infrastructure/2-cluster/providers.tf` (`gavinbunney/kubectl`,
  `~> 1.19`) and configured against the same kubeconfig.
- The idiom is **already used** in `shared-modules/operators/rabbitmq/main.tf`, which applies
  operator manifests with `kubectl_manifest` for the same reason.

So the change is to render the two ClusterIssuers as manifests and drop the Helm release, keeping
`depends_on = [helm_release.cert_manager]`.

## Migration notes

- **Module interface changes.** The module would need `kubectl` in its `providers` map, so every
  caller's `module "cert-manager"` block gains one line. Small, but it is a breaking change for
  callers.
- **The issuers get replaced, not updated.** Terraform destroys the Helm release and creates two
  manifests. That should be safe: the ACME account private keys live in the Secrets named by
  `privateKeySecretRef` (`letsencrypt-http-key`, `letsencrypt-dns-key`), which the release does not
  manage, so cert-manager should re-use the existing registration rather than register again. Worth
  confirming on a test cluster before doing it on a live one, because getting it wrong means new
  ACME accounts and fresh rate-limit counters.
- Existing certificates are unaffected either way; they are `Certificate` resources referencing the
  issuer by name, and the names do not change.

## If you would rather not change the provider surface

Pin the release (`version = "0.2.5"`) and, if the archived repo ever stops serving, mirror the
chart. That keeps today's behaviour and removes only the version ambiguity.

## Adjacent, not part of this

The cert-manager chart itself is unpinned - `# version = "v1.15.0"` is commented out in
`shared-modules/operators/cert-manager/main.tf`, so the release takes jetstack's latest, which
resolves to **v1.21.1** today, six minor versions past the number in that comment. An apply months
from now installs something other than what the plan described. Exposing a `chart_version` variable
with a default would let callers pin without forking the module.
