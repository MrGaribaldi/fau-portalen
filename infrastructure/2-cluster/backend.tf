# Remote state for stage 2, in Hetzner object storage.
#
# Same bucket and the same reasoning as stage 1: see 1-bootstrap/backend.tf and
# docs/stage-0-readiness.md. This stage writes the hcloud API token into the cluster as a
# Kubernetes Secret, so its state holds that token and deserves the same versioned,
# deletion-denied protection as the other stages.
#
# Credentials are NOT here. Pass them from the private runtime volume:
#   terraform init -backend-config=/infra-runtime/infrastructure/.backend.hcl
terraform {
  backend "s3" {
    bucket = "fau-tfstate"
    key    = "2-cluster/terraform.tfstate"
    region = "hel1"

    endpoints = {
      s3 = "https://hel1.your-objectstorage.com"
    }

    use_path_style = true
    use_lockfile   = true

    # Hetzner object storage is Ceph RGW, not AWS: skip the AWS-specific preflight checks.
    skip_credentials_validation = true
    skip_region_validation      = true
    skip_requesting_account_id  = true
    skip_metadata_api_check     = true
  }
}
