# Dev-only server config, merged with the -dev built-in config (docs/key-service-design.md
# §4.4). OpenBao refuses to create audit devices via the API by default (verified 27
# September 2026 against openbao/openbao:2.7.0: `bao audit enable` returns "cannot enable
# audit device via API; use declarative, config-based audit device management instead").
# Declaring the device here, applied on start and on SIGHUP, is the supported path — not the
# `unsafe_allow_api_audit_creation` escape hatch. Production's Helm config (Task 9,
# helm-values.yaml) declares the same stanza.
audit "file" "stdout" {
  description = "HMACed audit log to stdout, collected into Loki (docs/key-service-design.md §4.4)."
  options {
    file_path = "stdout"
  }
}
