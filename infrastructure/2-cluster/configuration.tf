# =====================================
# Stage 2 configuration
# =====================================
#
# FAU: adapted from infra-tools/infrastructure/2-cluster. Deliberate differences from
# upstream, all in this stage's own files; the shared modules are reused unchanged.
#
#   1. This pass installs ONLY the "cluster" module - Hetzner CCM, Hetzner CSI, the
#      coredns-custom ConfigMap and Stakater Reloader. Upstream's stage 2 also brings
#      ingress-nginx, cert-manager, cloudnativepg, rabbitmq, flux and the telemetry stack.
#      Each of those is held back for a reason rather than for tidiness:
#        - cert-manager needs persistent_outputs.cloudflare_api_token, which is empty:
#          FAU's edge and DNS are still open, see docs/edge-dns-assessment.md.
#        - flux needs the GitOps repository and the SOPS age key wiring, which is the one
#          thing that would make a key export outside Hetzner a real trigger (#3481).
#        - telemetry-services needs an alerting endpoint, which is exactly the open
#          question on #3442, and a db_memory_request decision belongs with cloudnativepg
#          (#3408).
#        - rabbitmq has no FAU use case in the MVP scope.
#      Installing the CCM alone is what clears the uninitialized taint and lets the three
#      Pending kube-system pods schedule, which is the immediate goal.
#
#   2. forward_dns_zones is empty. Upstream forwards "cluster.telemetry.local" to
#      10.30.0.10, a separate telemetry cluster that FAU does not have. The module still
#      creates the coredns-custom ConfigMap, just with no forward rules in it.
#
#   3. No alerting locals. Upstream's configuration.tf hardcodes a Slack channel, a public
#      ntfy.sh topic and an SMTP2GO smarthost. FAU dropped all three as upstream examples
#      (#3442), and the replacement is undecided, so nothing is carried over here.
#
#   4. certificate_email is not set. Upstream's ak@akanto.dk belongs to cert-manager, which
#      this pass does not install.
locals {
  # Hetzner CCM needs no DNS forwarding for FAU: there is no second cluster to reach.
  forward_dns_zones = {}
}
