terraform {
  required_version = ">= 1.15.0"

  required_providers {
    # https://registry.terraform.io/providers/hetznercloud/hcloud/latest/docs
    # Needed by the ingress-nginx module, which annotates the controller Service so the
    # CCM adopts the existing lb-fau load balancer. Pinned to the exact version the
    # module itself requires.
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "1.66.0"
    }

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

# FAU: upstream's stage 2 also declares kubectl and minio. Those belong to modules this root
# does not install (rabbitmq, telemetry), so they are left out until the module that needs
# them is added. hcloud was added on 10 September 2026 together with ingress-nginx.
#
# FAU divergence in the token source: upstream reads local.bootstrap_outputs.hcloud_token,
# but FAU's bootstrap outputs carry no such key - the token lives in persistent_outputs,
# which is where the cluster module already reads it. Using upstream's expression here
# would configure the provider with null and fail at the first API call.

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

provider "hcloud" {
  token = local.persistent_outputs.hcloud_token
}
