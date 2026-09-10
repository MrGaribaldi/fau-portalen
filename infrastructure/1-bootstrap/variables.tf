# Path to infra tools
#
# Local checkout in your own project, cloned to <project>/infrastructure/infra-tools:
#   infra_tools_path = "../infra-tools"
#   infra_tools_ref  = ""
#
# Straight from git, pinned to a branch or tag. 
# Note the single trailing slash on the path: it joins the literal "/shared-modules" to form "//", 
# which is how a git source separates the repository from a subdirectory within it.
#   infra_tools_path = "git::https://github.com/akantodevs/infra-tools.git/"
# Checkout specific branch
#   infra_tools_ref  = "?ref=version-4"
# FAU: this must be a RELATIVE path, unlike stage 0 which uses "/opt/infra-tools".
# The bootstrap/* modules reach their sibling shared-modules with "../" sources. An absolute
# path makes Terraform treat that directory as a self-contained module package, and a "../"
# source inside a package is rejected with "Local module path escapes module package". A
# relative path keeps every module in the root module's own package, where "../" is allowed.
# Both /workspace/infrastructure/1-bootstrap and /infra-runtime/infrastructure/1-bootstrap sit
# three levels below /, so the same value resolves to /opt/infra-tools in each.
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
