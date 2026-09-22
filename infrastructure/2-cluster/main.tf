# Stage 2 - cluster components.
# See configuration.tf for the FAU divergences from upstream and for local variables.
#
# Prerequisites, in order:
#   1. Stage 1 applied, all nodes Ready.               (done 9 September 2026)
#   2. The WireGuard tunnel up, or nothing here can
#      reach https://10.0.1.250:6443.                  (.agents/skills/fau-vpn/SKILL.md)
#   3. terraform init -backend-config=/infra-runtime/infrastructure/.backend.hcl
locals {
  persistent_outputs = jsondecode(file("../.config/persistent_outputs.json"))
  bootstrap_outputs  = jsondecode(file("../.config/bootstrap_outputs.json"))
}

# =========================================================
# PREPARE CLUSTER
# Hetzner CCM, Hetzner CSI, coredns-custom, Reloader.
#
# The CCM is the point of this pass. Every node carries
# node.cloudprovider.kubernetes.io/uninitialized=true:NoSchedule, the external
# cloud-provider handshake, and the CCM removes it as it initialises each node. Until then
# coredns, local-path-provisioner and metrics-server stay Pending.
#
# robot.enabled resolves to false because hcloud_robot_user is empty: FAU has no dedicated
# server, so no Robot credentials are registered. That is deliberate, see
# docs/planning-decisions.md.
# =========================================================
module "cluster" {
  source = "${var.infra_tools_path}/shared-modules/bootstrap/cluster${var.infra_tools_ref}"

  providers = {
    kubernetes = kubernetes
    helm       = helm
  }

  hcloud_token          = local.persistent_outputs.hcloud_token
  hcloud_ssh_key_name   = local.persistent_outputs.hcloud_ssh_key_name
  hcloud_robot_user     = local.persistent_outputs.hcloud_robot_user
  hcloud_robot_password = local.persistent_outputs.hcloud_robot_password
  network_backend_id    = local.bootstrap_outputs.network_backend_id
  forward_dns_zones     = local.forward_dns_zones
}

# =========================================================
# INGRESS-NGINX
# The controller Service is annotated for the Hetzner CCM, which adopts the load balancer
# named by bootstrap_outputs.load_balancer_name - lb-fau, created by stage 1 and already
# billed. So this pass hands a live resource to the CCM rather than creating one.
#
# replicas = 2 matches upstream. The controller's nodeSelector requires cloud=true and
# node-role=worker; all three nodes carry both, but vpn-router keeps a NoSchedule taint, so
# the two pods land on master and cworker-1. The chart spreads with
# whenUnsatisfiable = "ScheduleAnyway", so neither replica can be stranded Pending.
#
# The Service annotation uses-proxyprotocol = true means client addresses arrive over the
# PROXY protocol; ADR-001 requires the application to trust forwarded headers only from the
# configured proxy, and #3416/#3424 must configure both sides.
# =========================================================
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

# =========================================================
# CERT-MANAGER
# Requires ingress-nginx, because the module's letsencrypt-http ClusterIssuer hardcodes
# ingressClassName = "nginx" in its HTTP-01 solver.
#
# cloudflare_api_token is deliberately empty. FAU's 9 September certificate decision issues
# per hostname over HTTP-01 with no wildcard - FAU-er are addressed by path, never by
# subdomain - so the DNS-01 solver and its token are removed from the design, not pending.
# The upstream module creates the Secret and a letsencrypt-dns ClusterIssuer unconditionally;
# with an empty token both exist and the DNS issuer can never solve a challenge. Erik
# accepted that on #3485 as clutter rather than a fault. The one way it bites: an Ingress
# annotated cert-manager.io/cluster-issuer: letsencrypt-dns is accepted and never issues.
# =========================================================
module "cert-manager" {
  source = "${var.infra_tools_path}/shared-modules/operators/cert-manager${var.infra_tools_ref}"

  providers = {
    kubernetes = kubernetes
    helm       = helm
  }

  certificate_email    = local.certificate_email
  cloudflare_api_token = local.persistent_outputs.cloudflare_api_token

  depends_on = [module.cluster, module.ingress-nginx]
}

# =========================================================
# STAGING CLUSTERISSUER - FAU addition, no upstream equivalent
# Upstream defines only the production ACME endpoint, for both of its issuers. Production
# allows five duplicate certificates per week per hostname, and a misconfigured challenge
# loop burns that in minutes - after which issuance for the name is blocked for days. First
# HTTP-01 work therefore runs against staging, on the placeholder hostname, before the
# production issuer is used at all.
#
# Delivered through the same `raw` chart upstream uses for its own issuers: a ClusterIssuer
# is a cert-manager CRD, and kubernetes_manifest cannot plan a resource whose CRD does not
# yet exist. Inherited rather than chosen: charts.helm.sh/incubator is a deprecated Helm
# repository that upstream's issuer release already depends on.
# =========================================================
resource "helm_release" "fau_staging_issuer" {
  name       = "cert-manager-issuer-staging"
  chart      = "raw"
  repository = "https://charts.helm.sh/incubator"
  version    = "0.2.5"
  namespace  = "cert-manager"

  values = [
    yamlencode({
      resources = [
        {
          apiVersion = "cert-manager.io/v1"
          kind       = "ClusterIssuer"
          metadata = {
            name = "letsencrypt-staging"
          }
          spec = {
            acme = {
              server = "https://acme-staging-v02.api.letsencrypt.org/directory"
              email  = local.certificate_email
              privateKeySecretRef = {
                name = "letsencrypt-staging-key"
              }
              solvers = [
                {
                  http01 = {
                    ingress = {
                      ingressClassName = "nginx"
                    }
                  }
                }
              ]
            }
          }
        }
      ]
    })
  ]

  depends_on = [module.cert-manager]
}

