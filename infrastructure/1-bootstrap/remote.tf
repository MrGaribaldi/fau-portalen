# =====================================
# Remote servers (WireGuard-joined, off the private backend network)
# Orchestration lives in shared-modules/bootstrap/remote-servers; this stage just
# supplies the fleet definition + ports from configuration.tf.
# =====================================

module "bootstrap_remote" {
  source = "${var.infra_tools_path}/shared-modules/bootstrap/remote-servers${var.infra_tools_ref}"

  providers = {
    hcloud = hcloud
  }

  cluster_name          = local.persistent_outputs.cluster_name
  ssh_key_name          = local.persistent_outputs.hcloud_ssh_key_name
  admin_ssh_private_key = local.persistent_outputs.admin_ssh_private_key

  router_wg_private_key = local.persistent_outputs.remote_router_private_key
  router_wg_public_key  = local.persistent_outputs.remote_router_public_key

  vpn_router_external_ip = module.bootstrap_network.vpn_router_external_ip
  vpn_router_ssh_port    = local.public_ssh_port

  network_remote_subnet = local.network_remote_subnet
  wg_allowed_ips        = concat([local.network_backend_subnet], local.global_routes)
  remote_router_wg_port = local.remote_router_wg_port
  remote_ssh_port       = local.public_ssh_port
  remote_wg_listen_port = local.remote_wg_listen_port

  remote_servers = local.remote_servers

  depends_on = [module.bootstrap_network]
}
