# Dev-only server config, merged with the -dev built-in config (docs/key-service-design.md
# §4.4). OpenBao 2.7 blocks audit device creation via the API by default
# (unsafe_allow_api_audit_creation defaults to false, verified 27 September 2026 against
# openbao/openbao:2.7.0): configure.sh's `bao audit enable` would otherwise fail with
# "cannot enable audit device via API; use declarative, config-based audit device management
# instead". Allowed here so the same configure.sh works against the dev server; the
# production Helm chart (§4.1) declares its audit device in its own config instead and must
# never set this flag.
unsafe_allow_api_audit_creation = true
