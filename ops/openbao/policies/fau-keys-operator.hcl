# The deletion job (docs/key-service-design.md §5). It can destroy keys but never read data:
# no encrypt, decrypt or datakey.
path "transit/keys" { capabilities = ["list"] }
path "transit/keys/+" { capabilities = ["read", "delete"] }   # read = metadata only (soft_deleted); export is a separate path
path "transit/keys/+/config" { capabilities = ["update"] }
path "transit/keys/+/soft-delete" { capabilities = ["delete"] }
path "transit/keys/+/soft-delete-restore" { capabilities = ["update"] }
# The 7-day queue: OpenBao keeps no soft-delete timestamp, so the job records its own.
path "fau-keys-queue" { capabilities = ["list"] }
path "fau-keys-queue/+" { capabilities = ["create", "read", "update", "delete"] }
