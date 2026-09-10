# =====================================
# Network configuration
# =====================================
#
# FAU: adapted from infra-tools/infrastructure/1-bootstrap/configuration.tf.
# Deliberate differences from upstream, all in this file; the shared modules are reused
# unchanged:
#   1. All three servers are cx23 (2 vCPU, 4 GB, 40 GB) instead of cpx12 + cpx22 + cpx22.
#      Decided 9 September 2026, see docs/planning-decisions.md "Node choice: start on three
#      CX23". Identical nodes, 12 GB total, about EUR 16.47/month compute.
#   2. Upstream's "extra-firewall" resource is removed. It opened TCP 6000-6001 to
#      0.0.0.0/0 and ::/0 on the worker as a demo of the firewall_ids plumbing; FAU opens
#      no public ports beyond what the modules require.
#   3. The demo node label "extra-node-label=yes" is removed.
locals {
  cluster_domain               = "cluster.${local.persistent_outputs.cluster_name}.local"
  network_backend_subnet       = "10.0.0.0/14"
  network_default_gw           = "10.0.0.1"
  network_cloud_subnet         = "10.0.1.0/24"
  network_dedicated_subnet     = "10.0.2.0/24"
  network_vpn_client_subnet    = "10.0.200.0/24"
  network_remote_subnet        = "10.0.201.0/24"
  public_ssh_port              = 54322
  network_dedicated_vswitch_id = 0
  cluster_cidr                 = "10.1.0.0/16" # Dual stack: "10.1.0.0/16,fd42:10:1::/56"
  service_cidr                 = "10.2.0.0/16" # Dual stack: "10.2.0.0/16,fd42:10:2::/112"
}

# =====================================
# VPN Router config
# =====================================
locals {
  vpn_router_name         = "vpn-router"
  vpn_router_server_type  = "cx23" # FAU: was cpx12
  vpn_router_internal_ip  = "10.0.1.254"
  vpn_router_firewall_ids = []
  vpn_server_ip_cidr      = "10.0.200.1/24"
  vpn_devops_client_ip    = "10.0.200.2"
  vpn_router_use_ipv6     = false
  wireguard_listen_port   = 51890 # must fall within bootstrap/network var.wireguard_ports

  vpn_peers = [
    # Example peer configuration for connecting to telemetry cluster
    # {
    #   name        = "telemetry"
    #   public_key  = "<PEER_PUBLIC_KEY>"
    #   allowed_ips = "10.28.0.0/14" # CIDR for telemetry cluster (adjust as needed)
    #   endpoint    = "<PEER_ENDPOINT>:51890"
    # },
  ]
}

# =====================================
# K3S Master configuration
# =====================================
locals {
  k3s_master_internal_ip = "10.0.1.250"
  k3s_master_name        = "master"
  k3s_master_type        = "cx23"               # FAU: was cpx22
  k3s_master_node_labels = ["node-role=worker"] # Worker role to master allow scheduling pods on it (save costs)
}

# =====================================
# Cloud worker node configuration
# =====================================
locals {
  cloud_workers = [
    {
      name        = "cworker-1"
      internal_ip = "10.0.1.100"
      server_type = "cx23" # FAU: was cpx22
      public_ipv4 = true
    },
    # {
    #   name        = "cworker-2"
    #   internal_ip = "10.0.1.101"
    #   server_type = "cx23"
    #   public_ipv4 = true
    # },
  ]
}

# =====================================
# Remote server configuration
# =====================================
locals {
  remote_router_wg_port = 51891 # router wg1 listen port (within bootstrap/network wireguard_ports 51890-51899)
  remote_wg_listen_port = 51891 # Wireguard listen port for remote servers

  remote_servers = [
    # {
    #   name        = "remote-1"
    #   server_type = "cpx11"
    #   location    = "ash"
    #   wg_ip       = "10.0.201.10"
    #   public_ipv4 = true
    #   labels      = ["remote=true"]
    #   node_taints = []
    # },
  ]
}

# =====================================
# Load balancer configuration
# =====================================
locals {
  load_balancer_enabled = true
  load_balancer_name    = "lb-${local.persistent_outputs.cluster_name}"
  load_balancer_type    = "lb11"
}
