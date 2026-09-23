-- 0001: the schema contract. Creates the contract table and nothing else, so the
-- migration runner is proven end to end before any domain structure depends on it.
create table schema_contract (
  version    integer     primary key,
  applied_at timestamptz not null default now()
);

insert into schema_contract (version) values (1);
