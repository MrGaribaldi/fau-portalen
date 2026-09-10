# Bootstrap network and servers for the cluster
# See configuration.tf for local variables

locals {
  persistent_outputs = jsondecode(file("../.config/persistent_outputs.json"))
}

# Bootstrap network infrastructure
module "bootstrap_network" {
  source = "${var.infra_tools_path}/shared-modules/bootstrap/network${var.infra_tools_ref}"

  providers = {
    hcloud = hcloud
  }

  cluster_name                   = local.persistent_outputs.cluster_name
  hcloud_location                = local.persistent_outputs.hcloud_location
  hcloud_ssh_key_name            = local.persistent_outputs.hcloud_ssh_key_name
  admin_ssh_private_key          = local.persistent_outputs.admin_ssh_private_key
  hcloud_token                   = local.persistent_outputs.hcloud_token
  hcloud_robot_user              = local.persistent_outputs.hcloud_robot_user
  hcloud_robot_password          = local.persistent_outputs.hcloud_robot_password
  network_backend_subnet         = local.network_backend_subnet
  network_cloud_subnet           = local.network_cloud_subnet
  network_dedicated_subnet       = local.network_dedicated_subnet
  network_vpn_client_subnet      = local.network_vpn_client_subnet
  network_remote_subnet          = local.network_remote_subnet
  vpn_router_name                = local.vpn_router_name
  vpn_router_server_type         = local.vpn_router_server_type
  vpn_router_ssh_port            = local.public_ssh_port
  vpn_router_internal_ip         = local.vpn_router_internal_ip
  vpn_router_floating_ip_address = local.persistent_outputs.vpn_router_floating_ip_address
  vpn_router_floating_ip_id      = local.persistent_outputs.vpn_router_floating_ip_id
  vpn_router_firewall_ids        = local.vpn_router_firewall_ids
  network_dedicated_vswitch_id   = local.network_dedicated_vswitch_id
  load_balancer_enabled          = local.load_balancer_enabled
  load_balancer_name             = local.load_balancer_name
}

# =====================================
# Global routes
# =====================================
# Non-backend CIDRs reachable through the vpn-router's WireGuard fabric (e.g. the
# telemetry cluster). Derived from vpn_peers so a peer and its routes can never drift
# apart: add a peer, every cloud and remote server routes its networks to the router.
# Peer CIDRs that fall inside the backend supernet (e.g. vpn-client host routes in
# 10.0.200.0/24) are filtered out — those are already reachable via the backend route.
locals {
  backend_mask = tonumber(split("/", local.network_backend_subnet)[1])
  backend_base = cidrhost(local.network_backend_subnet, 0)

  global_routes = distinct([
    for c in flatten([for p in local.vpn_peers : [for x in split(",", p.allowed_ips) : trimspace(x)]]) :
    c
    if cidrhost(format("%s/%d", split("/", c)[0], local.backend_mask), 0) != local.backend_base
  ])
}

# Create servers
module "bootstrap_servers" {
  source = "${var.infra_tools_path}/shared-modules/bootstrap/servers${var.infra_tools_ref}"

  providers = {
    hcloud = hcloud
  }

  cluster_name           = local.persistent_outputs.cluster_name
  hcloud_location        = local.persistent_outputs.hcloud_location
  vpn_router_external_ip = module.bootstrap_network.vpn_router_external_ip
  vpn_router_ssh_port    = local.public_ssh_port
  hcloud_ssh_key_name    = local.persistent_outputs.hcloud_ssh_key_name
  admin_ssh_private_key  = local.persistent_outputs.admin_ssh_private_key
  network_backend_id     = module.bootstrap_network.network_backend_id
  network_backend_subnet = local.network_backend_subnet
  network_default_gw     = local.network_default_gw
  global_routes          = local.global_routes
  server_ssh_port        = local.public_ssh_port
  cloud_workers          = local.cloud_workers
  depends_on             = [module.bootstrap_network]
}

# Output all bootstrap values to a file for subsequent stage handoffs
resource "local_file" "bootstrap_output" {
  filename = "../.config/bootstrap_outputs.json"
  content = jsonencode({
    cluster_domain              = local.cluster_domain
    network_backend_id          = module.bootstrap_network.network_backend_id
    network_backend_subnet      = local.network_backend_subnet
    network_default_gw          = local.network_default_gw
    network_cloud_subnet_id     = module.bootstrap_network.network_cloud_subnet_id
    network_cloud_subnet        = local.network_cloud_subnet
    network_dedicated_subnet_id = module.bootstrap_network.network_dedicated_subnet_id
    network_dedicated_subnet    = local.network_dedicated_subnet
    vpn_router_internal_ip      = local.vpn_router_internal_ip
    vpn_router_server_id        = module.bootstrap_network.vpn_router_server_id
    vpn_router_ssh_port         = local.public_ssh_port
    vpn_router_external_ip      = module.bootstrap_network.vpn_router_external_ip
    vpn_router_external_ip_ipv6 = module.bootstrap_network.vpn_router_external_ip_ipv6
    load_balancer_name          = local.load_balancer_name
    load_balancer_ip            = module.bootstrap_network.load_balancer_ip
    k3s_server_ip               = local.k3s_master_internal_ip
    k3s_node_token              = module.k3s_master_install.k3s_node_token
    k3s_backup_bucket_name      = local.persistent_outputs.k3s_backup_bucket_name
    cloud_workers               = local.cloud_workers
    remote_servers              = module.bootstrap_remote.remote_servers
    db_backup_bucket_name       = local.persistent_outputs.db_backup_bucket_name
  })
}
