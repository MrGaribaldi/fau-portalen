# Path to infra tools
#
# FAU: this must be a RELATIVE path, for the same reason as stage 1 - the bootstrap/*
# modules reach their siblings with "../" sources, and an absolute path makes Terraform
# treat that directory as a self-contained module package where "../" is rejected with
# "Local module path escapes module package". Both /workspace/infrastructure/2-cluster and
# /infra-runtime/infrastructure/2-cluster sit three levels below /, so the same value
# resolves to /opt/infra-tools in each.
variable "infra_tools_path" {
  description = "Location of the shared modules: a path to an infra-tools checkout relative to this stage directory, or a git source prefix ending in '/'."
  type        = string
  const       = true
  default     = "../../../opt/infra-tools"
}

variable "infra_tools_ref" {
  description = "Git ref suffix appended to module sources, e.g. \"?ref=version-4\". Must be empty when infra_tools_path is a local path."
  type        = string
  const       = true
  default     = ""
}
