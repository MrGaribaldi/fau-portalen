terraform {
  required_version = ">= 1.15.0"

  required_providers {
    # https://registry.terraform.io/providers/hetznercloud/hcloud/latest/docs
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "1.66.0"
    }

    # https://registry.terraform.io/providers/aminueza/minio/latest/docs
    minio = {
      source  = "aminueza/minio"
      version = "3.12.0"
    }
  }
}

provider "hcloud" {
  token = local.persistent_outputs.hcloud_token
}

provider "minio" {
  minio_user     = local.persistent_outputs.s3_access_key
  minio_password = local.persistent_outputs.s3_secret_key
  minio_server   = local.persistent_outputs.s3_endpoint
  minio_region   = local.persistent_outputs.s3_region
  minio_ssl      = true
}
