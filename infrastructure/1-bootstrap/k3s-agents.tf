# =====================================
# K3S agents (worker / vpn-router / remote node joins)
# =====================================
# Joining a node is a pure SSH operation (the k3s-agent module is null_resource only),
# so it belongs in the same stage as the master install rather than in 2-cluster.
#
# NOTE on readiness: master + agents run `cloud-provider=external` and the master sets
# `disable-cloud-controller: true`, so freshly joined nodes carry the
# `node.cloudprovider.kubernetes.io/uninitialized` taint and stay NotReady until the
# Hetzner CCM (installed by module.cluster in stage 2) initializes them. That is expected
# here — there is intentionally no depends_on module.cluster (it lives in stage 2 and
# needs the k8s API). Nodes self-heal once stage 2 runs.

# ========================================================
# VPN ROUTER AGENT
# Allow access to internal services
# ========================================================
module "vpn-router-agent" {
  source = "${var.infra_tools_path}/shared-modules/k3s-agent${var.infra_tools_ref}"

  server_ip             = local.vpn_router_internal_ip
  server_port           = local.public_ssh_port
  bastion_ip            = module.bootstrap_network.vpn_router_external_ip
  bastion_ssh_port      = local.public_ssh_port
  admin_ssh_private_key = local.persistent_outputs.admin_ssh_private_key
  k3s_server_ip         = local.k3s_master_internal_ip
  node_token            = module.k3s_master_install.k3s_node_token
  node_labels           = ["vpn-gw=true"]
  node_taints           = ["node-role.kubernetes.io/vpn-router=true:NoSchedule"]

  depends_on = [module.bootstrap_network, module.k3s_master_install, null_resource.reset_vpn_router_node_password]
}

# ========================================================
# WORKER AGENTS
# ========================================================
module "worker-agents" {
  source = "${var.infra_tools_path}/shared-modules/k3s-agent${var.infra_tools_ref}"

  for_each              = { for worker in local.cloud_workers : worker.name => worker }
  server_ip             = each.value.internal_ip
  server_port           = local.public_ssh_port
  bastion_ip            = module.bootstrap_network.vpn_router_external_ip
  bastion_ssh_port      = local.public_ssh_port
  admin_ssh_private_key = local.persistent_outputs.admin_ssh_private_key
  k3s_server_ip         = local.k3s_master_internal_ip
  node_token            = module.k3s_master_install.k3s_node_token
  node_labels           = try(each.value.labels, [])
  node_taints           = try(each.value.node_taints, [])

  depends_on = [module.bootstrap_servers, module.k3s_master_install, null_resource.reset_worker_node_password]
}

# ========================================================
# REMOTE AGENTS
# k3s agents on remote servers
# ========================================================
module "remote-agents" {
  source = "${var.infra_tools_path}/shared-modules/k3s-agent${var.infra_tools_ref}"

  for_each = { for s in module.bootstrap_remote.remote_servers : s.name => s }

  server_ip             = each.value.external_ipv4
  server_port           = each.value.ssh_port
  node_ip               = each.value.wg_ip
  node_external_ip      = each.value.external_ipv4
  flannel_iface         = each.value.flannel_iface
  admin_ssh_private_key = local.persistent_outputs.admin_ssh_private_key
  k3s_server_ip         = local.k3s_master_internal_ip
  node_token            = module.k3s_master_install.k3s_node_token
  # exclude-from-external-load-balancers: the CCM otherwise tries to register these nodes as
  # Hetzner LB targets, which fails — their only reachable address is the WireGuard IP, not a
  # routable Hetzner private/public target. Remote nodes are never valid LB backends.
  node_labels = concat(["node.kubernetes.io/exclude-from-external-load-balancers=true"], try(each.value.labels, []))
  node_taints = concat(["node-role.kubernetes.io/remote=true:NoSchedule"], try(each.value.node_taints, []))
  ccm_managed = false

  depends_on = [module.bootstrap_remote, module.k3s_master_install, null_resource.reset_remote_node_password]
}

# =====================================
# Node-password reset (recreate remediation)
# =====================================
# When an agent host is rebuilt it generates a NEW /etc/rancher/node/password, but the
# master still holds the <node>.node-password.k3s secret pinned at the node's original
# registration -> k3s rejects the mismatch and the agent can never rejoin. These resources
# delete the stale secret on the master *before* the agent re-registers, so k3s re-pins the
# new password (trust-on-first-use). Keyed on the hcloud server id, so they fire ONLY when a
# host is actually replaced; --ignore-not-found makes the first-ever join a no-op.
# Master access reuses the same vpn-router-bastion SSH path as module.k3s_master_install.

resource "null_resource" "reset_vpn_router_node_password" {
  triggers = {
    server_id = module.bootstrap_network.vpn_router_server_id
  }

  provisioner "remote-exec" {
    inline = [
      "k3s kubectl -n kube-system delete secret vpn-router.node-password.k3s --ignore-not-found",
    ]

    connection {
      host                = local.k3s_master_internal_ip
      port                = local.public_ssh_port
      user                = "root"
      private_key         = local.persistent_outputs.admin_ssh_private_key
      bastion_host        = module.bootstrap_network.vpn_router_external_ip
      bastion_port        = local.public_ssh_port
      bastion_user        = "root"
      bastion_private_key = local.persistent_outputs.admin_ssh_private_key
    }
  }

  depends_on = [module.k3s_master_install]
}

resource "null_resource" "reset_worker_node_password" {
  for_each = { for worker in local.cloud_workers : worker.name => worker }

  triggers = {
    server_id = module.bootstrap_servers.worker_ids[each.key]
  }

  # On reprovision the hcloud server_id changes but the node keeps its name, so the
  # Hetzner CCM never re-initializes the existing (already-initialized) Node object:
  # spec.providerID (immutable) and ExternalIP stay pinned to the destroyed server, and
  # a recycled public IP can collide with another node's ExternalIP. Deleting the Node
  # object forces the CCM to re-initialize it from scratch when the rebuilt agent rejoins.
  # Both deletions are required and independent: the secret so the agent can re-register
  # (trust-on-first-use), the Node object so the CCM re-initializes providerID/ExternalIP.
  # Runs BEFORE the new agent joins via module.worker-agents' depends_on on this resource.
  provisioner "remote-exec" {
    inline = [
      "k3s kubectl -n kube-system delete secret ${each.key}.node-password.k3s --ignore-not-found",
      "k3s kubectl delete node ${each.key} --ignore-not-found",
    ]

    connection {
      host                = local.k3s_master_internal_ip
      port                = local.public_ssh_port
      user                = "root"
      private_key         = local.persistent_outputs.admin_ssh_private_key
      bastion_host        = module.bootstrap_network.vpn_router_external_ip
      bastion_port        = local.public_ssh_port
      bastion_user        = "root"
      bastion_private_key = local.persistent_outputs.admin_ssh_private_key
    }
  }

  depends_on = [module.k3s_master_install, module.bootstrap_servers]
}

resource "null_resource" "reset_remote_node_password" {
  for_each = { for s in module.bootstrap_remote.remote_servers : s.name => s }

  triggers = {
    server_id = each.value.id
  }

  provisioner "remote-exec" {
    inline = [
      "k3s kubectl -n kube-system delete secret ${each.key}.node-password.k3s --ignore-not-found",
    ]

    connection {
      host                = local.k3s_master_internal_ip
      port                = local.public_ssh_port
      user                = "root"
      private_key         = local.persistent_outputs.admin_ssh_private_key
      bastion_host        = module.bootstrap_network.vpn_router_external_ip
      bastion_port        = local.public_ssh_port
      bastion_user        = "root"
      bastion_private_key = local.persistent_outputs.admin_ssh_private_key
    }
  }

  depends_on = [module.k3s_master_install, module.bootstrap_remote]
}

# =====================================
# Remote Node object cleanup (destroy-time)
# =====================================
# Remote nodes are self-managed (ccm_managed = false), so the Hetzner CCM will NOT delete
# their Node objects when the servers are destroyed -> a stale NotReady node would linger.
# This destroy-time provisioner removes the Node object from the master when the remote
# server goes away (one removed from remote_servers, or terraform destroy). Destroy
# provisioners may reference only self.*, so all connection inputs live in triggers (same
# pattern as the k3s-master uninstall step). depends_on ensures this is destroyed -> runs
# BEFORE the master/bastion are torn down.

resource "null_resource" "remote_node_cleanup" {
  for_each = { for s in module.bootstrap_remote.remote_servers : s.name => s }

  triggers = {
    node_name             = each.key
    master_ip             = local.k3s_master_internal_ip
    ssh_port              = local.public_ssh_port
    bastion_ip            = module.bootstrap_network.vpn_router_external_ip
    admin_ssh_private_key = local.persistent_outputs.admin_ssh_private_key
  }

  provisioner "remote-exec" {
    when   = destroy
    inline = ["k3s kubectl delete node ${self.triggers.node_name} --ignore-not-found"]

    connection {
      host                = self.triggers.master_ip
      port                = self.triggers.ssh_port
      user                = "root"
      private_key         = self.triggers.admin_ssh_private_key
      bastion_host        = self.triggers.bastion_ip
      bastion_port        = self.triggers.ssh_port
      bastion_user        = "root"
      bastion_private_key = self.triggers.admin_ssh_private_key
    }
  }

  depends_on = [module.k3s_master_install, module.bootstrap_network]
}
