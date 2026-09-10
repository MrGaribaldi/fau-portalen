# Path to infra tools
#
# FAU uses the read-only local mount of the infra-tools checkout that agent-box provides
# at /opt/infra-tools. The shared modules are consumed unchanged from there.
#
# The alternative is a commit-pinned git source, which needs GitHub read access at init time:
#   infra_tools_path = "git::https://github.com/akantodevs/infra-tools.git/"
#   infra_tools_ref  = "?ref=6c301e15e3ee83a9797971046ffe1bc9b8593ca3"
# Note the single trailing slash on the git path: it joins the literal "/shared-modules" to
# form "//", which is how a git source separates the repository from a subdirectory within it.
variable "infra_tools_path" {
  description = "Location of the shared modules: a path to an infra-tools checkout relative to this stage directory, or a git source prefix ending in '/'."
  type        = string
  const       = true
  default     = "/opt/infra-tools"
}

variable "infra_tools_ref" {
  description = "Git ref suffix appended to module sources, e.g. \"?ref=version-4\". Must be empty when infra_tools_path is a local path."
  type        = string
  const       = true
  default     = ""
}

# S3 credentials
variable "s3_access_key" {
  description = "Access key for S3"
  type        = string
  sensitive   = true
}

variable "s3_secret_key" {
  description = "Secret key for S3"
  type        = string
  sensitive   = true
}

# Hetzner credentials
variable "hcloud_token" {
  description = "Hetzner cloud API token"
  type        = string
  default     = ""
  sensitive   = true
}

variable "hcloud_robot_user" {
  description = "Hetzner robot user for API access"
  type        = string
  default     = ""
  sensitive   = true
}

variable "hcloud_robot_password" {
  description = "Hetzner robot password for API access"
  type        = string
  default     = ""
  sensitive   = true
}

# Cloudflare API token
variable "cloudflare_api_token" {
  description = "Cloudflare API token"
  type        = string
  default     = ""
  sensitive   = true
}

# Grafana admin password
variable "grafana_admin_password" {
  description = "Admin password for Grafana"
  type        = string
  sensitive   = true
}

# ALERTING
# Handed to stage 2 via persistent_outputs.json, the same way grafana_admin_password is.
variable "slack_webhook_url" {
  description = "Slack incoming webhook URL, used for both Flux notifications and Alertmanager"
  type        = string
  sensitive   = true
  default     = ""
}

variable "ntfy_token" {
  description = "Bearer token for the ntfy topic Alertmanager posts to"
  type        = string
  sensitive   = true
  default     = ""
}

variable "smtp_auth_password" {
  description = "SMTP password used for warning-severity alert emails"
  type        = string
  sensitive   = true
  default     = ""
}

variable "grafana_dashboards_repo_token" {
  description = "GitHub PAT for the Grafana dashboards repository"
  type        = string
  sensitive   = true
  default     = ""
}
