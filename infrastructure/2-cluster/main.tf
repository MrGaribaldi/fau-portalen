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
