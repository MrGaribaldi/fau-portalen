terraform {
  required_version = ">= 1.15.0"

  required_providers {
    # https://registry.terraform.io/providers/hashicorp/kubernetes/latest/docs
    kubernetes = {
      source  = "hashicorp/kubernetes"
      version = "~> 2.37.1"
    }

    # https://registry.terraform.io/providers/hashicorp/helm/latest/docs
    helm = {
      source  = "hashicorp/helm"
      version = "~> 3.0.2"
    }
  }
}

# FAU: upstream's stage 2 also declares hcloud, kubectl and minio. Those belong to modules
# this pass does not install (ingress-nginx, rabbitmq, telemetry), so they are left out
# until the module that needs them is added. Fewer providers, smaller lock file, and no
# credentials configured for a provider nothing uses yet.

# Both providers reach the cluster at https://10.0.1.250:6443, which is private. The
# WireGuard tunnel must be up before plan or apply: see .agents/skills/fau-vpn/SKILL.md.
provider "kubernetes" {
  config_path = "../.config/kubeconfig.yaml"
}

provider "helm" {
  kubernetes = {
    config_path = "../.config/kubeconfig.yaml"
  }
}
