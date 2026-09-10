# =====================================
# K3S master (control-plane node)
# =====================================
# Master's own firewall (workers keep their own default-fw in bootstrap/servers).
# Outbound-allow only; inbound is default-deny (admin access is via the WG fabric).
resource "hcloud_firewall" "master" {
  name = "master-fw"

  rule {
    direction       = "out"
    protocol        = "tcp"
    port            = "1-65535"
    destination_ips = ["0.0.0.0/0", "::/0"]
    description     = "Allow all outgoing TCP"
  }

  rule {
    direction       = "out"
    protocol        = "udp"
    port            = "1-65535"
    destination_ips = ["0.0.0.0/0", "::/0"]
    description     = "Allow all outgoing UDP"
  }

  rule {
    direction       = "out"
    protocol        = "icmp"
    destination_ips = ["0.0.0.0/0", "::/0"]
    description     = "Allow outgoing ping"
  }
}

# Create k3s master cloud server instance
module "k3s_master_server" {
  source = "${var.infra_tools_path}/shared-modules/cloud-server${var.infra_tools_ref}"

  providers = {
    hcloud = hcloud
  }

  name                   = local.k3s_master_name
  cluster_name           = local.persistent_outputs.cluster_name
  firewall_ids           = [hcloud_firewall.master.id]
  type                   = local.k3s_master_type
  public_ipv4            = true
  location               = local.persistent_outputs.hcloud_location
  ssh_key_name           = local.persistent_outputs.hcloud_ssh_key_name
  network_backend_id     = module.bootstrap_network.network_backend_id
  network_backend_subnet = local.network_backend_subnet
  global_routes          = local.global_routes
  internal_ip            = local.k3s_master_internal_ip
  default_gateway_ip     = local.network_default_gw
  admin_ssh_private_key  = local.persistent_outputs.admin_ssh_private_key
  server_ssh_port        = local.public_ssh_port
  bastion_ip             = module.bootstrap_network.vpn_router_external_ip
  bastion_ssh_port       = local.public_ssh_port
}

# Install k3s server
module "k3s_master_install" {
  source = "${var.infra_tools_path}/shared-modules/k3s-master${var.infra_tools_ref}"

  cluster_name          = local.persistent_outputs.cluster_name
  cluster_domain        = local.cluster_domain
  server_name           = local.k3s_master_name
  server_ip             = local.k3s_master_internal_ip
  server_ssh_port       = local.public_ssh_port
  cluster_cidr          = local.cluster_cidr
  service_cidr          = local.service_cidr
  node_labels           = local.k3s_master_node_labels
  s3_access_key         = local.persistent_outputs.s3_access_key
  s3_secret_key         = local.persistent_outputs.s3_secret_key
  s3_endpoint           = local.persistent_outputs.s3_endpoint
  s3_region             = local.persistent_outputs.s3_region
  k3s_backup_bucket     = local.persistent_outputs.k3s_backup_bucket_name
  admin_ssh_private_key = local.persistent_outputs.admin_ssh_private_key
  bastion_ip            = module.bootstrap_network.vpn_router_external_ip
  bastion_ssh_port      = local.public_ssh_port

  depends_on = [
    module.k3s_master_server,
  ]
}
