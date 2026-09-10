# =====================================
# WireGuard VPN fabric
# =====================================
module "wireguard_devops_client" {
  source = "${var.infra_tools_path}/shared-modules/wireguard_client${var.infra_tools_ref}"

  name              = local.persistent_outputs.cluster_name
  client_ip         = local.vpn_devops_client_ip
  private_key       = local.persistent_outputs.vpn_devops_private_key
  public_key        = local.persistent_outputs.vpn_devops_public_key
  server_public_key = local.persistent_outputs.vpn_server_public_key
  # Preserve the original fallback: floating IP if set, else router external IP.
  endpoint_address = coalesce(local.persistent_outputs.vpn_router_floating_ip_address, module.bootstrap_network.vpn_router_external_ip)
  server_port      = local.wireguard_listen_port
  allowed_ips      = local.network_backend_subnet
}

module "wireguard_router" {
  source = "${var.infra_tools_path}/shared-modules/wireguard_service${var.infra_tools_ref}"

  host                  = module.bootstrap_network.vpn_router_external_ip
  ssh_port              = local.public_ssh_port
  admin_ssh_private_key = local.persistent_outputs.admin_ssh_private_key
  interface_name        = "wg0"
  address_cidr          = local.vpn_server_ip_cidr
  listen_port           = local.wireguard_listen_port
  private_key           = local.persistent_outputs.vpn_server_private_key
  public_key            = local.persistent_outputs.vpn_server_public_key

  # NAT all wg0 peers to the cluster. wg0 is the admin/management fabric; peers
  # here are hidden behind the router (cluster sees the router IP). Routed
  # site-to-site peers (e.g. remote-server) get their own WireGuard interface
  # rather than a NAT carve-out of this subnet.
  post_up = [
    "iptables -t nat -C POSTROUTING -s ${local.network_vpn_client_subnet} -j MASQUERADE || iptables -t nat -A POSTROUTING -s ${local.network_vpn_client_subnet} -j MASQUERADE",
  ]
  post_down = [
    "iptables -t nat -D POSTROUTING -s ${local.network_vpn_client_subnet} -j MASQUERADE",
  ]

  peers = concat(
    [module.wireguard_devops_client.peer],
    local.vpn_peers,
  )

  # Load-bearing, not redundant: the implicit dep via vpn_router_external_ip only
  # waits for the server object. This also waits for the module's
  # null_resource.wait_for_vpn_router_init (cloud-init gate that installs the
  # wireguard package) before wireguard_service's SSH provisioner runs.
  depends_on = [module.bootstrap_network]
}
