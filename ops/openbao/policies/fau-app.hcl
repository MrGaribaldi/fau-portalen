# The FAU backend (docs/key-service-design.md §4.3). One path segment only (+): a `*` here
# would also grant transit/keys/<name>/config, /rotate, /trim and /soft-delete (verified
# 27 September 2026). Changing this file needs the policy matrix in crates/keys/tests.
path "transit/keys/+" { capabilities = ["update"] }         # create a key; never read, list or delete
path "transit/datakey/plaintext/+" { capabilities = ["update"] }
path "transit/datakey/wrapped/+" { capabilities = ["update"] }
path "transit/encrypt/+" { capabilities = ["update"] }
path "transit/decrypt/+" { capabilities = ["update"] }
