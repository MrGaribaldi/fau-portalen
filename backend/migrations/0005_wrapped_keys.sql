-- #3506 (docs/key-service-design.md §3.2): OpenBao-wrapped data keys, one per unit. A wrapped
-- key is useless without its transit key in OpenBao, which exists at exactly the granularity
-- it must be shredded at, so a copy in any backup of this table is useless once that key is
-- destroyed. No usable key material is ever stored here.
create table wrapped_keys (
  tenant_id    uuid        not null references tenants (id),
  unit         text        not null check (unit in ('record', 'document', 'chat')),
  -- Document id or YYYY-MM; null for 'record'. Not `subject`, which ADR-003 decision 3
  -- reserves for identity_mappings.
  scope        text,
  -- OpenBao transit ciphertext, `vault:v<n>:<base64>`.
  wrapped_key  bytea       not null,
  created_at   timestamptz not null default now(),
  -- The key is (tenant_id, unit, scope), spec §3.2. It cannot be a primary key, which would
  -- force scope not null; `nulls not distinct` makes one constraint cover the record key too.
  constraint wrapped_keys_one_per_unit unique nulls not distinct (tenant_id, unit, scope),
  constraint wrapped_keys_scope_matches_unit check ((unit = 'record') = (scope is null)),
  constraint wrapped_keys_is_transit_ciphertext
    check (substring(wrapped_key from 1 for 7) = 'vault:v'::bytea and octet_length(wrapped_key) <= 512)
);
grant select, insert, delete on wrapped_keys to fau_app;

-- The #3418 messages are now OpenBao transit ciphertext (vault:v1: + base64), about 2,713 bytes
-- for 500 characters, so the 0003 bound of 2200 no longer fits.
alter table access_requests rename column sealed_message to encrypted_message;
alter table access_requests drop constraint access_request_sealed_message_is_bounded;
alter table access_requests add constraint access_request_encrypted_message_is_bounded
  check (octet_length(encrypted_message) <= 4096);
alter table invitations drop constraint invitation_encrypted_message_is_bounded;
alter table invitations add constraint invitation_encrypted_message_is_bounded
  check (octet_length(encrypted_message) <= 4096);

insert into schema_contract (version) values (5);
