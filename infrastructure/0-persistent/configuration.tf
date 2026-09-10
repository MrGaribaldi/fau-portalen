# Overall persistent configuration for the infrastructure. 
# This includes values that are shared across multiple modules and should be consistent throughout the entire infrastructure setup. 
# These values are also stored in the persistent outputs to ensure they can be accessed by other modules during bootstrapping and future operations.
#
# FAU: adapted from infra-tools/infrastructure/0-persistent/configuration.tf.
# Only the names and the location differ from upstream; the shared modules are reused unchanged.
locals {
  cluster_name                  = "fau"
  hcloud_location               = "hel1"
  hcloud_ssh_key_name           = "fau-admin"
  s3_endpoint                   = "hel1.your-objectstorage.com"
  s3_region                     = "hel1"
  k3s_backup_bucket_name        = "${local.cluster_name}-k3s-backup"
  db_backup_bucket_name         = "${local.cluster_name}-db-backup"
  vpn_router_assign_floating_ip = false
}
