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

    # https://registry.terraform.io/providers/jackivanov/x25519/latest/docs
    x25519 = {
      source  = "jackivanov/x25519"
      version = "~> 1.0.8"
    }

    # https://registry.terraform.io/providers/clementblaise/age/latest/docs
    age = {
      source  = "clementblaise/age"
      version = "~> 0.1.1"
    }
  }
}

provider "hcloud" {
  token = var.hcloud_token
}

provider "minio" {
  minio_user     = var.s3_access_key
  minio_password = var.s3_secret_key
  minio_server   = local.s3_endpoint
  minio_region   = local.s3_region
  minio_ssl      = true
}
