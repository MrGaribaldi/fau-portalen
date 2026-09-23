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

# =====================================
# FAU addition: off-node etcd snapshot retention
# =====================================
# k3s keeps S3 snapshot retention in its own setting, etcd-s3-retention, which defaults to 5.
# Upstream's k3s-config.tpl.yaml sets only etcd-snapshot-retention (168), so master kept a week
# of hourly snapshots while the private fau-k3s-backup bucket kept five - the configuration
# behaving as written rather than a defect. Findings and decision: #3488 and
# docs/cluster-hygiene-findings-2026-09-10.md.
#
# k3s v1.35.3+ inherits the general value when the S3 flag is unset (k3s-io/k3s#13770); this
# cluster runs v1.35.2, and declaring the value is preferable to inheriting it in any case.
#
# Delivered as a config.yaml.d drop-in rather than by editing upstream's template: that template
# is part of the k3s-master module's config_hash, so changing it re-runs the entire control-plane
# install script. This writes one file and restarts k3s.
#
# If k3s ever fails to start after this, remove
# /etc/rancher/k3s/config.yaml.d/10-fau-s3-retention.yaml and `systemctl restart k3s`.
locals {
  # One scalar key, absent from config.yaml, so the drop-in merge cannot conflict with it.
  k3s_s3_retention_dropin = "etcd-s3-retention: 168\n"
}

resource "null_resource" "k3s_master_s3_retention" {
  triggers = {
    dropin          = local.k3s_s3_retention_dropin
    server_ip       = local.k3s_master_internal_ip
    server_ssh_port = local.public_ssh_port
  }

  provisioner "file" {
    content     = local.k3s_s3_retention_dropin
    destination = "/tmp/10-fau-s3-retention.yaml"

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

  provisioner "remote-exec" {
    inline = [
      "set -eu",
      "install -d -m 0700 /etc/rancher/k3s/config.yaml.d",
      "install -m 0600 /tmp/10-fau-s3-retention.yaml /etc/rancher/k3s/config.yaml.d/10-fau-s3-retention.yaml",
      "rm -f /tmp/10-fau-s3-retention.yaml",
      "systemctl restart k3s",
      # Do not report success until the API server answers again.
      "for i in $(seq 1 60); do k3s kubectl get --raw /readyz >/dev/null 2>&1 && break; sleep 5; done",
      "k3s kubectl get --raw /readyz",
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

