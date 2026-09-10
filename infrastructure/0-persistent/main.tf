# This project owns the persistent resources (ssh/vpn key, s3 buckets, tokens, etc.) for the infrastructure.
# These resources are created first and their outputs are used as inputs for the other stages. 
# This allows us to keep all the persistent values in one place and easily reference them in subsequent stages.

# ==============================================
# SSH Keys
# ==============================================
# Generate a new SSH key pair
resource "tls_private_key" "generated" {
  algorithm = "RSA"
  rsa_bits  = 4096
}

# Create an SSH key in Hetzner Cloud using the generated public key
resource "hcloud_ssh_key" "generated" {
  name       = local.hcloud_ssh_key_name
  public_key = tls_private_key.generated.public_key_openssh
}

# Create local files for the private and public keys
resource "local_file" "private_key_file" {
  filename        = "../.config/ssh_key/id_ed25519"
  content         = tls_private_key.generated.private_key_pem
  file_permission = "0600"
}

resource "local_file" "public_key_file" {
  filename        = "../.config/ssh_key/id_ed25519.pub"
  content         = tls_private_key.generated.public_key_openssh
  file_permission = "0644"
}

# ==============================================
# WireGuard VPN Keys
# ==============================================

# Generate a WireGuard key pair for the vpn router
resource "x25519_private_key" "vpn_router" {}

# Generate a WireGuard key pair for the devops client
resource "x25519_private_key" "devops" {}

# Generate a WireGuard key pair for the remote-server router interface (wg1)
resource "x25519_private_key" "remote_router" {}

# ===============================================
# Setup SOPS for repo secrets management
# ===============================================
# FAU divergence from upstream: sops_filename is "../.sops.yaml", not "../../.sops.yaml".
# Terraform runs in /infra-runtime/infrastructure/0-persistent, so upstream's path would
# resolve to /infra-runtime/.sops.yaml -- a root-owned 0755 directory the agent cannot
# write to. Writing it beside the stage dirs keeps it inside the private runtime root.
# The file holds only the age PUBLIC key, so it is the one generated artifact that may be
# copied back into /workspace. The shared module itself is used unchanged.
module "sops" {
  source        = "${var.infra_tools_path}/shared-modules/sops${var.infra_tools_ref}"
  sops_filename = "../.sops.yaml"
}

# ==============================================
# Floating IP for VPN router 
# ==============================================
resource "hcloud_floating_ip" "vpn_router_ip" {
  count         = local.vpn_router_assign_floating_ip ? 1 : 0
  name          = "vpn-router-ipv4"
  type          = "ipv4"
  home_location = local.hcloud_location
}

# Output all persistent values to a file for subsequent stage handoffs
# FAU divergence from upstream: file_permission pinned to 0600. This file carries the
# hcloud token, S3 keys, the admin SSH private key, the WireGuard private keys and the
# SOPS age secret key; upstream leaves it at local_file's 0644 default.
resource "local_file" "persistent_output" {
  filename        = "../.config/persistent_outputs.json"
  file_permission = "0600"
  content = jsonencode({
    cluster_name                   = local.cluster_name
    hcloud_location                = local.hcloud_location
    hcloud_token                   = var.hcloud_token
    hcloud_robot_user              = var.hcloud_robot_user
    hcloud_robot_password          = var.hcloud_robot_password
    s3_endpoint                    = local.s3_endpoint
    s3_region                      = local.s3_region
    s3_access_key                  = var.s3_access_key
    s3_secret_key                  = var.s3_secret_key
    hcloud_ssh_key_name            = local.hcloud_ssh_key_name
    admin_ssh_public_key           = tls_private_key.generated.public_key_openssh
    admin_ssh_private_key          = tls_private_key.generated.private_key_pem
    k3s_backup_bucket_name         = local.k3s_backup_bucket_name
    db_backup_bucket_name          = local.db_backup_bucket_name
    vpn_server_private_key         = x25519_private_key.vpn_router.private_key
    vpn_server_public_key          = x25519_private_key.vpn_router.public_key
    vpn_devops_private_key         = x25519_private_key.devops.private_key
    vpn_devops_public_key          = x25519_private_key.devops.public_key
    remote_router_private_key      = x25519_private_key.remote_router.private_key
    remote_router_public_key       = x25519_private_key.remote_router.public_key
    vpn_router_floating_ip_address = local.vpn_router_assign_floating_ip ? hcloud_floating_ip.vpn_router_ip[0].ip_address : null
    vpn_router_floating_ip_id      = local.vpn_router_assign_floating_ip ? hcloud_floating_ip.vpn_router_ip[0].id : null
    cloudflare_api_token           = var.cloudflare_api_token
    grafana_admin_password         = var.grafana_admin_password
    slack_webhook_url              = var.slack_webhook_url
    ntfy_token                     = var.ntfy_token
    smtp_auth_password             = var.smtp_auth_password
    grafana_dashboards_repo_token  = var.grafana_dashboards_repo_token
    age_sops_secret_key            = module.sops.age_sops_secret_key
    age_sops_public_key            = module.sops.age_sops_public_key
  })
}
