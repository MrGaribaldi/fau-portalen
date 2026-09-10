# Remote state for stage 1, in Hetzner object storage.
#
# FAU divergence from upstream: infra-tools keeps local state in every stage. Stage 0 is
# moved to a remote backend because its state is the only copy of the generated admin SSH
# key, the three WireGuard private keys and the SOPS age key. Stages 1-3 read
# persistent_outputs.json rather than remote state, so stage 0 can move on its own.
#
# The fau-tfstate bucket is deliberately NOT managed by Terraform: a bucket cannot hold the
# state that manages it. It was created out of band on 8 September 2026 and must not be
# added to any root configuration. See docs/stage-0-readiness.md.
#
# Locking uses S3-native conditional writes (use_lockfile), verified against this endpoint:
# a second PUT with If-None-Match:* returns 412. No DynamoDB equivalent is needed.
#
# Credentials are NOT here. Pass them from the private runtime volume:
#   terraform init -backend-config=/infra-runtime/infrastructure/.backend.hcl
terraform {
  backend "s3" {
    bucket = "fau-tfstate"
    key    = "1-bootstrap/terraform.tfstate"
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
