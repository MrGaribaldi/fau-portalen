# Application Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the FAU Rust backend skeleton — one binary with `serve`, `migrate` and
`--version`, the schema-contract mechanism, the first two migrations, the privilege split, health
probes, structured logging, the error contract, graceful shutdown, a production image and a local
Compose stack — with integration tests against real PostgreSQL.

**Architecture:** A Cargo workspace of three crates. `domain` holds entities and rules and declares
neither `axum` nor `sqlx`, so ADR-001's "the domain does not depend on HTTP or UI" is enforced by
the dependency graph. `persistence` owns sqlx, the pool and every transaction boundary.
`app` wires axum and configuration and produces the `fau` binary. Migrations are ordered immutable
SQL applied by a separate `migrate` invocation of the same binary, never by `serve`.

**Tech Stack:** Rust 1.98.1, axum 0.8, tokio 1, sqlx 0.8 (postgres, runtime-tokio, rustls),
tracing + tracing-subscriber JSON, clap 4, serde 1, uuid 1 (v7), jiff 0.2 (Europe/Oslo dates),
PostgreSQL 17.5, Docker multi-stage build, Docker Compose.

**Spec:** `docs/app-foundation-design.md` (approved 22 September 2026, #3416).
Supporting: `docs/repo-container-contract.md` (ADR-001, #3411),
`docs/tenant-role-history-design.md` (#3412),
`docs/identity-and-encryption.md` (ADR-003, #3484),
`docs/localisation-design.md` (#3439).

---

## Decisions this plan makes

The spec's section 16 leaves five things "open for implementation". This plan closes them. Each is
a judgment call made without Erik present on the night of 22 September 2026, and each is cheap to
reverse.

| Open item | Decision | Reason |
| --- | --- | --- |
| Base image digests | `rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e`, `debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251` (multi-arch index digests; the amd64 manifests first written here, `c49256cb…` and `f3034a6e…`, are children of these indexes — changed 23 September 2026 so the image also builds on arm64) | Resolved from the registry 22 September 2026. Rust 1.98.1 matches the toolchain already in the agent box, so local `cargo test` and the image build agree. |
| PostgreSQL patch version | `postgres:17.5-bookworm@sha256:2088c1744625793a8a89118d2dee63fb121139141ff0ff53bd72e63bb6089d0d` | Exactly the version infra-tools' `psql-cluster` module runs (`ghcr.io/cloudnative-pg/postgresql:17.5`). Matching the patch, not just the major, removes a class of "works locally" surprise. |
| Static placeholder: binary or file | Embedded in the binary with `include_str!` | The runtime stage has a read-only root filesystem and readiness must not depend on the filesystem. One less failure mode, and #3422 replaces it wholesale anyway. |
| `lock_timeout` value | 10 000 ms, overridable with `MIGRATION_LOCK_TIMEOUT_MS` | See "The advisory-lock nuance" below (corrected 23 September 2026). |
| Initial `schema_contract` minimum | 2 | The binary requires the identity and tenancy spine that `0002` creates. Declaring 1 would let it serve a database it cannot use. |

### The advisory-lock nuance

The spec's test table says *"Migration lock held beyond the bound | Fails rather than hanging —
`lock_timeout` is set on the migration connection."*

> **Corrected 23 September 2026.** This section first claimed that `lock_timeout` does not apply to
> advisory locks. That was wrong: the Task 4+5 review reproduced a `pg_advisory_lock` wait being
> cancelled by `lock_timeout=500` after 0.52 s. `lock_timeout` alone would therefore bound sqlx's
> own migration lock, and the spec line is literally true. The outer lock below is kept for a
> different reason: the wait for another migrator and the DDL bound are different quantities.
> Queueing behind a concurrent `migrate` may reasonably take 30 s; a migration holding table locks
> against live traffic should give up after 10 s. One `lock_timeout` cannot express both, and the
> outer lock also yields a named `LockUnavailable` error instead of a generic SQLSTATE 55P03.

This plan therefore does both:

1. Sets `lock_timeout` on the migration connection, which bounds the DDL inside each migration —
   the case where a migration waits on a table lock held by a long-running query.
2. Takes **our own outer advisory lock** with `pg_try_advisory_lock` in a bounded poll before
   handing off to sqlx's migrator. Every FAU migration process contends for the same outer lock, so
   only one reaches sqlx at a time, and the wait has a deadline we control and can test.

`statement_timeout` is deliberately **not** used to bound the lock: it would also kill a legitimately
long migration, converting a slow deploy into a failed one.

### Where this plan departs from a literal reading of the spec

- **`.env.example` at the repository root already exists** and belongs to the agent-box stack
  (`FAU_HOST_PATH`, `TTYD_PASSWORD`). Rather than add a second file and break the acceptance command
  `cp .env.example .env`, this plan **appends an application section** to the existing file. One
  `.env` serves both stacks; Compose ignores variables a given stack does not use. Task 13 documents
  this for anyone who already has a `.env`.
- **`/workspace/.dockerignore` excludes everything** (`*`) and re-includes only the favro-cli source
  for `Dockerfile.agent`. The application image would therefore build with no `backend/` in its
  context. Task 12 re-includes `backend/` explicitly. The deny-by-default shape is kept — it is what
  stops `.env` reaching a build — so only the paths the app image needs are added.
- **`docs/tenant-role-history-design.md` is in the repository.** The task brief said the approved
  #3412 model existed only as a Favro attachment. It was downloaded from #3412 and is byte-identical
  to the committed file, so the committed file is the source used here.
- **Documents are out of scope**, so migration `0002` carries no `document.language`, no document
  `type`/`model_version` and no task rows, even though `CLAUDE.md` lists them among "the first
  migration" requirements. The spec (later, and approved) scopes `0002` to identity and tenancy and
  puts documents on #3419/#3490. Those columns belong to the migration that creates `documents`.
  The requirement that survives into `0002` is the negative one: **no key material in any table**.

---

## Global Constraints

Every task's requirements implicitly include this section. Values are copied verbatim from the
spec and ADRs.

**Language and copy**
- All technical content in English: code, comments, SQL, documentation, commit messages, error
  codes. (`CLAUDE.md`.)
- API errors carry a **stable error code, bounded parameters and the request ID — never display
  text**. An endpoint that returns Norwegian prose silently breaks localisation. (Spec §11.)

**Architecture**
- `crates/domain` declares neither `axum` nor `sqlx` in `Cargo.toml`. Enforced by a test. (Spec §2.)
- `persistence` owns transaction boundaries. (Spec §2.)
- `fau serve` **never performs DDL**. (Spec §3, ADR-001.)

**Schema**
- Every reference between tenant data uses the composite key `(tenant_id, id)`, so a cross-tenant
  reference is rejected by the database. (Spec §5 property 1, #3412.)
- **Several identity mappings per account** — `(issuer, subject)` is the primary key of
  `identity_mappings`, and `account_id` is *not* unique there. (Spec §5 property 2, ADR-003 dec. 3.)
- Role periods are half-open `[starts_on, ends_on_exclusive)`, the end is **not null**, and
  `check (starts_on < ends_on_exclusive)`. Dates are local calendar dates in `Europe/Oslo`; event
  timestamps are `timestamptz` in UTC. (Spec §5 property 3.)
- **No key material of any kind in any application table.** Enforced by a schema review test.
  (Spec §5, ADR-003 dec. 5.)
- The provider's `sub` is never a primary key, never a foreign key, and appears in no table other
  than `identity_mappings`. (ADR-003 dec. 3.)
- `accounts.locale` nullable; `tenants.default_locale` not null default `nb-NO`. Nothing may
  hard-code two locales or assume Latin collation. (#3439.)
- `accounts.retention_months` not null default 3. (ADR-003 dec. 6a.)

**Configuration**
- Required for `serve`: `APP_ENV`, `HTTP_BIND`, `PUBLIC_BASE_URL`, `DATABASE_URL`,
  `DB_POOL_MAX_CONNECTIONS`, `LOG_LEVEL`. Required for `migrate`: `MIGRATION_DATABASE_URL` only.
- `HTTP_BIND` defaults to `0.0.0.0:8000`; `LOG_LEVEL` defaults to `info`.
- Invalid or missing required configuration fails at startup with the **variable name, never its
  value**, and a non-zero exit.
- A merely unreachable database is **not** a configuration error: readiness goes false with bounded
  retry and the process stays up. (Spec §4.)
- `fau --version` reads no configuration and touches no secret. (Spec §3.)

**HTTP**
- Serves `/health/live`, `/health/ready` and a static placeholder at `/`. `/api/v1/` and `/app/`
  are reserved and carry no routes yet.
- Unknown API routes return 404 and **never** fall back to HTML. (Spec §8.)
- `/health/live` touches the process only — no database, mail, RabbitMQ, S3 or OTLP. (Spec §9.)
- `/health/ready` requires local initialisation complete, a database check with a **one-second
  timeout**, and a schema contract at or above the binary's minimum; 503 otherwise; result cached
  briefly. (Spec §9.)

**Logging**
- JSON to stdout is the only log transport. **Logs are not exported over OTLP** — Alloy would
  collect each line twice. OTLP stays reserved for traces. (Spec §10.)
- Every line carries timestamp, level, service version, request ID, **route template** (from axum's
  `MatchedPath`, never the URI), status and duration.
- Never logged: query strings, email addresses, cookies, `Authorization`, magic-link tokens, SQL
  parameters, document content. (Spec §10.)
- The `level` field uses `tracing-subscriber`'s conventional names so Alloy's `stage.json` promotes
  it without configuration on either side.

**Process and container**
- SIGTERM sets readiness false, stops accepting new work, allows in-flight requests up to **25
  seconds**, then exits. An aborted transaction is never acknowledged to the client. (Spec §12.)
- Multi-stage image, base images pinned **by digest**. Runtime stage carries the binary, CA
  certificates and `migrations/` and nothing else. Non-root, read-only root filesystem, tmpfs
  `/tmp`. (Spec §13.)

**Migrations**
- `_sqlx_migrations` answers *which files have run*; a changed checksum on an applied migration is
  an error. `schema_contract` answers *whether this binary can serve this database*, and the
  application **tolerates a higher version** so additive migrations can roll out before new pods.
  (Spec §5, ADR-001 expand/contract.)
- Migrations are immutable once applied.

**Testing**
- Integration tests run against real PostgreSQL. Each test gets **its own database created from a
  template and dropped afterwards** — not a wrapping transaction, because the code under test
  manages its own transactions. (Spec §15.)
- Tests are written before the code they exercise.

**Out of scope — do not build** (Spec §1): frontend build pipeline, any UI beyond the static
placeholder, authentication, documents/folders/tags, the collaboration authority, row-level
security, mail delivery beyond a local test adapter.

---

## File Structure

```text
backend/
  Cargo.toml                        workspace manifest, shared dependency versions
  Cargo.lock                        committed; pins every transitive version
  rust-toolchain.toml               pins 1.98.1
  crates/
    domain/
      Cargo.toml                    no axum, no sqlx — enforced by test
      src/lib.rs                    re-exports
      src/ids.rs                    AccountId, TenantId, MembershipId, RoleId … UUIDv7 newtypes
      src/locale.rs                 Locale (BCP 47), NB_NO default, parse/validate
      src/role_period.rs            RolePeriod half-open interval, Europe/Oslo calendar dates
      src/error_code.rs             ErrorCode enum — stable codes, no display text
      src/schema_contract.rs        ContractVersion, MINIMUM_CONTRACT_VERSION = 2, compatibility rule
    persistence/
      Cargo.toml                    sqlx, tokio
      src/lib.rs
      src/pool.rs                   build pool from DATABASE_URL + DB_POOL_MAX_CONNECTIONS
      src/migrate.rs                bounded outer advisory lock + sqlx Migrator
      src/contract.rs               read max(version) from schema_contract
      src/health.rs                 db_check with 1s timeout
    app/
      Cargo.toml                    axum, tower-http, tracing-subscriber, clap
      src/main.rs                   clap CLI: serve | migrate | --version
      src/config.rs                 typed env loading, redaction, variable-name-only errors
      src/telemetry.rs              tracing JSON subscriber to stdout
      src/shutdown.rs               SIGTERM/SIGINT → readiness false → 25s drain
      src/readiness.rs              ReadinessState, cached probe result
      src/http/mod.rs               router assembly, reserved prefixes, 404 rules
      src/http/health.rs            /health/live, /health/ready
      src/http/error.rs             ApiError → JSON {code, params, request_id}
      src/http/request_context.rs   request-id middleware + access log with MatchedPath
      src/http/placeholder.rs       include_str! of placeholder.html
      src/http/placeholder.html     Bokmål holding page, no JS, no secrets
  migrations/
    0001_schema_contract.sql
    0002_identity_and_tenancy.sql
  db/
    roles.sql                       creates fau_migrate and fau_app; precondition for 0002's grants
  tests/
    common/mod.rs                   ephemeral-database harness, template management
    cli.rs                          --version, migrate exit codes
    config.rs                       required/invalid configuration, no value in message
    migrations.rs                   checksum, concurrency, lock bound, idempotent re-run
    schema_spine.rs                 composite FK, role-period check, identity mappings, grants
    schema_review.rs                no key material in any table; runtime role has no DDL
    contract_gate.rs                below minimum refuses, above minimum serves
    http_surface.rs                 live, placeholder, 404 rules, error contract
    logging.rs                      sentinel secrets never appear in captured output
    readiness.rs                    db down → 503 → back → 200, no restart loop
    shutdown.rs                     SIGTERM drains in-flight, no acknowledged write lost
Dockerfile                          application image (Dockerfile.agent stays untouched)
compose.yaml                        db, migrate, app, mail — the application stack
.dockerignore                       modified: re-include backend/
.env.example                        modified: application section appended
docs/app-foundation-operations.md   upgrade path for an existing volume, role provisioning
```

Split by responsibility, not layer: configuration and its validation live together; the migration
runner and its lock live together; the schema contract exists twice on purpose — the *rule* in
`domain` (pure, testable without a database) and the *read* in `persistence`.

---

### Task 1: Workspace skeleton and the version contract

**Files:**
- Create: `backend/Cargo.toml`, `backend/rust-toolchain.toml`
- Create: `backend/crates/domain/Cargo.toml`, `backend/crates/domain/src/lib.rs`
- Create: `backend/crates/persistence/Cargo.toml`, `backend/crates/persistence/src/lib.rs`
- Create: `backend/crates/app/Cargo.toml`, `backend/crates/app/src/main.rs`
- Test: `backend/tests/cli.rs`, `backend/crates/domain/tests/dependency_boundary.rs`

**Interfaces:**
- Produces: binary `fau`; `fau --version` prints `fau <semver> (<git-rev-or-unknown>)` to stdout,
  exit 0. Build revision comes from the `FAU_BUILD_REVISION` build-time environment variable,
  falling back to `unknown` — the Dockerfile supplies it, `cargo build` locally does not.

- [ ] **Step 1: Write the failing tests**

`backend/tests/cli.rs`:

```rust
use std::process::Command;

fn fau() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fau"))
}

#[test]
fn version_prints_build_revision_and_exits_zero() {
    let out = fau().arg("--version").output().expect("run fau --version");
    assert!(out.status.success(), "status was {:?}", out.status);
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("fau "), "unexpected output: {stdout}");
}

#[test]
fn version_reads_no_configuration() {
    // Spec section 3: --version reads no configuration and touches no secret.
    // With every required variable absent it must still succeed.
    let out = fau()
        .arg("--version")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .output()
        .expect("run fau --version");
    assert!(out.status.success(), "status was {:?}", out.status);
}

#[test]
fn unknown_subcommand_exits_non_zero() {
    let out = fau().arg("frobnicate").output().expect("run fau");
    assert!(!out.status.success());
}
```

`backend/crates/domain/tests/dependency_boundary.rs`:

```rust
/// ADR-001: the domain does not depend on HTTP or UI. Spec section 2 makes this a
/// property of the dependency graph rather than of memory, so assert on the manifest.
#[test]
fn domain_manifest_declares_no_http_or_sql_dependency() {
    let manifest = include_str!("../Cargo.toml");
    for forbidden in ["axum", "sqlx", "tower", "hyper", "reqwest"] {
        assert!(
            !manifest.contains(forbidden),
            "domain/Cargo.toml must not declare {forbidden}"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd backend && cargo test`
Expected: FAIL — no such package.

- [ ] **Step 3: Create the workspace**

`backend/rust-toolchain.toml`:

```toml
[toolchain]
channel = "1.98.1"
components = ["rustfmt", "clippy"]
```

`backend/Cargo.toml` — a virtual workspace with a `[workspace.dependencies]` table so every crate
shares one version of each dependency. Members: `crates/domain`, `crates/persistence`, `crates/app`.
Declare here: `tokio` (features `rt-multi-thread`, `macros`, `signal`, `time`), `axum` 0.8,
`tower` 0.5, `tower-http` 0.6 (`trace`, `catch-panic`), `sqlx` 0.8 (default-features off;
`runtime-tokio`, `tls-rustls`, `postgres`, `uuid`, `time`, `macros` off — no compile-time query
checking, because that would require a live database to build the image), `serde` 1 (`derive`),
`serde_json` 1, `tracing` 0.1, `tracing-subscriber` 0.3 (`json`, `env-filter`), `clap` 4
(`derive`), `uuid` 1 (`v7`, `serde`), `jiff` 0.2, `thiserror` 2, `anyhow` 1.
`[workspace.package]` carries `version = "0.1.0"`, `edition = "2021"`, `license = "proprietary"`.

`backend/crates/app/Cargo.toml` sets `[[bin]] name = "fau"`, `path = "src/main.rs"`.

- [ ] **Step 4: Implement the CLI shell**

`backend/crates/app/src/main.rs` — clap `Parser` with `#[command(version = build_revision())]`
is not usable in a const context, so print the version by hand:

```rust
const BUILD_REVISION: &str = match option_env!("FAU_BUILD_REVISION") {
    Some(rev) => rev,
    None => "unknown",
};

#[derive(clap::Parser)]
#[command(name = "fau", disable_version_flag = true)]
struct Cli {
    /// Print the build revision and exit. Reads no configuration.
    #[arg(long, global = true)]
    version: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Serve HTTP. Never performs DDL.
    Serve,
    /// Apply pending migrations and exit.
    Migrate,
}
```

`main` checks `cli.version` first and returns before any configuration is read. `Serve` and
`Migrate` are stubbed with `todo!()` for now — Task 2 onward fills them. Exit codes go through a
single `fn main() -> std::process::ExitCode`.

- [ ] **Step 5: Run the tests**

Run: `cd backend && cargo test --test cli && cargo test -p fau-domain`
Expected: PASS.

- [ ] **Step 6: Commit** (blocked — see "Handover" at the end of this plan; leave in the working tree)

---

### Task 2: Configuration loading and validation

**Files:**
- Create: `backend/crates/app/src/config.rs`
- Modify: `backend/crates/app/src/main.rs`
- Test: `backend/tests/config.rs`

**Interfaces:**
- Produces:
  - `pub struct ServeConfig { app_env: AppEnv, http_bind: SocketAddr, public_base_url: Url,
    database_url: Secret<String>, db_pool_max_connections: u32, log_level: String }`
  - `pub struct MigrateConfig { migration_database_url: Secret<String>, lock_timeout_ms: u64,
    lock_wait_ms: u64 }`
  - `pub enum AppEnv { Development, Test, Production }`
  - `ServeConfig::from_env() -> Result<Self, ConfigError>` and `MigrateConfig::from_env()`
  - `pub struct ConfigError { pub variable: &'static str, pub problem: ConfigProblem }` whose
    `Display` is `configuration variable {variable} is {problem}` and **never** includes the value.
  - `Secret<T>` whose `Debug` and `Display` both render `[redacted]`.

- [ ] **Step 1: Write the failing tests**

```rust
// backend/tests/config.rs
use std::process::Command;

fn fau_with(env: &[(&str, &str)], arg: &str) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_fau"));
    cmd.arg(arg).env_clear().env("PATH", std::env::var("PATH").unwrap_or_default());
    for (k, v) in env { cmd.env(k, v); }
    cmd.output().expect("run fau")
}

const SERVE_ENV: &[(&str, &str)] = &[
    ("APP_ENV", "test"),
    ("HTTP_BIND", "127.0.0.1:0"),
    ("PUBLIC_BASE_URL", "http://localhost:8000"),
    ("DATABASE_URL", "postgres://u:hunter2@localhost:5432/fau"),
    ("DB_POOL_MAX_CONNECTIONS", "5"),
    ("LOG_LEVEL", "info"),
];

#[test]
fn missing_required_variable_names_it_and_exits_non_zero() {
    let env: Vec<_> = SERVE_ENV.iter().copied()
        .filter(|(k, _)| *k != "PUBLIC_BASE_URL").collect();
    let out = fau_with(&env, "serve");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("PUBLIC_BASE_URL"), "stderr was: {stderr}");
}

#[test]
fn invalid_value_is_never_echoed() {
    let mut env = SERVE_ENV.to_vec();
    env.retain(|(k, _)| *k != "DB_POOL_MAX_CONNECTIONS");
    env.push(("DB_POOL_MAX_CONNECTIONS", "banana-sentinel"));
    let out = fau_with(&env, "serve");
    assert!(!out.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(combined.contains("DB_POOL_MAX_CONNECTIONS"));
    assert!(!combined.contains("banana-sentinel"), "value leaked: {combined}");
}

#[test]
fn database_url_never_appears_in_output() {
    // The password inside DATABASE_URL is the most likely accidental leak.
    let env: Vec<_> = SERVE_ENV.iter().copied()
        .filter(|(k, _)| *k != "PUBLIC_BASE_URL").collect();
    let out = fau_with(&env, "serve");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!combined.contains("hunter2"), "password leaked: {combined}");
}

#[test]
fn migrate_does_not_require_serve_configuration() {
    // Spec section 4: migrate requires only MIGRATION_DATABASE_URL.
    // An unreachable host is a connection failure, not a configuration failure —
    // the message must not name PUBLIC_BASE_URL or DATABASE_URL.
    let out = fau_with(
        &[("MIGRATION_DATABASE_URL", "postgres://u:p@127.0.0.1:1/none")],
        "migrate",
    );
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("PUBLIC_BASE_URL"), "stderr was: {stderr}");
}

#[test]
fn http_bind_and_log_level_have_defaults() {
    let env: Vec<_> = SERVE_ENV.iter().copied()
        .filter(|(k, _)| *k != "HTTP_BIND" && *k != "LOG_LEVEL").collect();
    // Reaches the database-connection stage rather than failing on configuration.
    let out = fau_with(&env, "serve");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("HTTP_BIND"), "stderr was: {stderr}");
    assert!(!stderr.contains("LOG_LEVEL"), "stderr was: {stderr}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test config`
Expected: FAIL — `todo!()` panic or missing module.

- [ ] **Step 3: Implement `config.rs`**

`Secret<T>` wraps a value, exposes `expose(&self) -> &T`, and implements `Debug`/`Display` as
`[redacted]`. No `Serialize`.

Each variable is read through one helper so the redaction rule cannot be forgotten:

```rust
fn required(name: &'static str) -> Result<String, ConfigError> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        Ok(_) => Err(ConfigError { variable: name, problem: ConfigProblem::Empty }),
        Err(_) => Err(ConfigError { variable: name, problem: ConfigProblem::Missing }),
    }
}

fn parsed<T: std::str::FromStr>(name: &'static str, raw: &str) -> Result<T, ConfigError> {
    // The parse error is discarded on purpose: several FromStr implementations
    // quote the offending input, which is exactly what must not reach the log.
    raw.parse().map_err(|_| ConfigError { variable: name, problem: ConfigProblem::Invalid })
}
```

Validation rules: `APP_ENV` is one of `development`/`test`/`production`; `PUBLIC_BASE_URL` must
parse as a URL and must be HTTPS when `APP_ENV=production` (explicit localhost allowed otherwise);
`DB_POOL_MAX_CONNECTIONS` must be a positive integer. Telemetry and mail variables are read into an
`accepted_but_unused` set and ignored, per spec §4.

Wire `main.rs` so the configuration error path writes `Display` to stderr and returns
`ExitCode::FAILURE`, before any logging subscriber is installed.

- [ ] **Step 4: Run to verify the tests pass**

Run: `cd backend && cargo test --test config`
Expected: PASS.

- [ ] **Step 5: Leave in the working tree**

---

### Task 3: Integration test harness — ephemeral PostgreSQL databases

**Files:**
- Create: `backend/tests/common/mod.rs`
- Create: `backend/db/roles.sql` (stub; Task 5 fills it)
- Test: `backend/tests/harness_selftest.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub struct TestDb { pub name: String, admin_url: String }`
  - `TestDb::fresh().await -> TestDb` — creates an empty database, no migrations applied.
  - `TestDb::migrated().await -> TestDb` — creates a database from the migrated template.
  - `TestDb::url(&self) -> String` — a `DATABASE_URL` for the runtime role `fau_app`.
  - `TestDb::migration_url(&self) -> String` — a `MIGRATION_DATABASE_URL` for `fau_migrate`.
  - `TestDb::admin_pool(&self) -> PgPool` — superuser pool for assertions and direct SQL.
  - `Drop` for `TestDb` drops the database with `WITH (FORCE)`.
  - `pub fn admin_url() -> String` — from `TEST_DATABASE_URL`, default
    `postgres://postgres:postgres@127.0.0.1:5433/postgres`.

Port 5433, not 5432: the harness talks to the Compose `db` service published on the host, and 5433
keeps it clear of any PostgreSQL an operator already runs.

- [ ] **Step 1: Write the failing self-test**

```rust
// backend/tests/harness_selftest.rs
mod common;
use common::TestDb;

#[tokio::test]
async fn fresh_database_is_empty_and_isolated() {
    let a = TestDb::fresh().await;
    let b = TestDb::fresh().await;
    assert_ne!(a.name, b.name);

    sqlx::query("create table probe (id int primary key)")
        .execute(&a.admin_pool()).await.unwrap();

    let in_b: Option<String> = sqlx::query_scalar("select to_regclass('probe')::text")
        .fetch_one(&b.admin_pool()).await.unwrap();
    assert!(in_b.is_none(), "databases are not isolated");
}

#[tokio::test]
async fn dropped_database_is_gone() {
    let name = { let db = TestDb::fresh().await; db.name.clone() };
    // Drop ran at end of scope.
    let pool = common::admin_pool().await;
    let exists: bool = sqlx::query_scalar(
        "select exists(select 1 from pg_database where datname = $1)")
        .bind(&name).fetch_one(&pool).await.unwrap();
    assert!(!exists, "database {name} was left behind");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test harness_selftest`
Expected: FAIL — no `common` module. (If it fails to *connect*, start the stack first; Task 13
documents `docker compose up -d db`.)

- [ ] **Step 3: Implement the harness**

Template management is the only subtle part. Several test binaries run concurrently, so building
the template must happen exactly once across processes:

```rust
const TEMPLATE: &str = "fau_test_template";
// Arbitrary but fixed: two processes must choose the same key to contend on.
const TEMPLATE_LOCK: i64 = 0x0FAU_0001;

async fn ensure_template() {
    let pool = admin_pool().await;
    // Session-level advisory lock on the maintenance database, held while building.
    sqlx::query("select pg_advisory_lock($1)").bind(TEMPLATE_LOCK)
        .execute(&pool).await.unwrap();

    let exists: bool = sqlx::query_scalar(
        "select exists(select 1 from pg_database where datname = $1)")
        .bind(TEMPLATE).fetch_one(&pool).await.unwrap();

    if !exists {
        sqlx::query(&format!(r#"create database "{TEMPLATE}""#))
            .execute(&pool).await.unwrap();
        apply_roles_and_migrations(&database_url_for(TEMPLATE)).await;
    }

    sqlx::query("select pg_advisory_unlock($1)").bind(TEMPLATE_LOCK)
        .execute(&pool).await.unwrap();
}
```

`create database … template …` fails if any session is connected to the template, so
`apply_roles_and_migrations` must close its pool before returning — call `pool.close().await`
explicitly rather than relying on drop order.

`Drop` cannot await. Use a small blocking drop:

```rust
impl Drop for TestDb {
    fn drop(&mut self) {
        let (url, name) = (self.admin_url.clone(), self.name.clone());
        // A dedicated thread: we may be inside a tokio runtime that is shutting down.
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all().build().expect("drop runtime");
            rt.block_on(async {
                if let Ok(pool) = sqlx::PgPool::connect(&url).await {
                    let _ = sqlx::query(&format!(r#"drop database if exists "{name}" with (force)"#))
                        .execute(&pool).await;
                }
            });
        }).join();
    }
}
```

Database names are `fau_test_` plus a UUIDv7 with hyphens removed, so they sort by creation time
and a leaked one is easy to spot.

- [ ] **Step 4: Run the self-test**

Run: `cd backend && cargo test --test harness_selftest`
Expected: PASS.

- [ ] **Step 5: Leave in the working tree**

---

### Task 4: Migration runner, bounded lock, and migration 0001

**Files:**
- Create: `backend/migrations/0001_schema_contract.sql`
- Create: `backend/crates/persistence/src/migrate.rs`, `backend/crates/persistence/src/pool.rs`
- Modify: `backend/crates/app/src/main.rs` (wire `Command::Migrate`)
- Test: `backend/tests/migrations.rs`

**Interfaces:**
- Consumes: `MigrateConfig` (Task 2), `TestDb` (Task 3).
- Produces:
  - `pub async fn run_migrations(cfg: &MigrationSettings) -> Result<Applied, MigrateError>`
  - `pub struct MigrationSettings { pub url: String, pub lock_timeout_ms: u64, pub lock_wait_ms: u64 }`
  - `pub struct Applied { pub applied: usize, pub already_current: bool }`
  - `pub enum MigrateError { LockUnavailable, ChecksumMismatch { version: i64 }, Connect(..), Sql(..) }`
  - `fau migrate` exits 0 on success, non-zero on every error.

- [ ] **Step 1: Write the failing tests**

```rust
// backend/tests/migrations.rs
mod common;
use common::TestDb;

#[tokio::test]
async fn migrate_applies_and_is_idempotent() {
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    let first = common::run_fau_migrate(&db).await;
    assert!(first.status.success(), "{:?}", first);

    let second = common::run_fau_migrate(&db).await;
    assert!(second.status.success(), "re-running migrate must succeed");

    let version: i32 = sqlx::query_scalar("select max(version) from schema_contract")
        .fetch_one(&db.admin_pool()).await.unwrap();
    assert_eq!(version, 2, "contract version after all migrations");
}

#[tokio::test]
async fn altered_checksum_on_an_applied_migration_fails() {
    let db = TestDb::migrated().await;
    // Corrupt the recorded checksum, which is how sqlx detects an edited file.
    sqlx::query("update _sqlx_migrations set checksum = decode('00', 'hex') where version = 1")
        .execute(&db.admin_pool()).await.unwrap();

    let out = common::run_fau_migrate(&db).await;
    assert!(!out.status.success(), "migrate must refuse a changed checksum");
}

#[tokio::test]
async fn concurrent_migrate_processes_serialise() {
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    let (a, b) = tokio::join!(common::run_fau_migrate(&db), common::run_fau_migrate(&db));
    assert!(a.status.success() && b.status.success(),
        "both must succeed: one applies, the other finds nothing to do");

    // Exactly one row per migration proves neither applied the same file twice.
    let rows: i64 = sqlx::query_scalar("select count(*) from _sqlx_migrations")
        .fetch_one(&db.admin_pool()).await.unwrap();
    assert_eq!(rows, 2);
}

#[tokio::test]
async fn migrate_fails_rather_than_hanging_when_the_lock_is_held() {
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;

    // Hold the outer lock from an independent session.
    let holder = db.admin_pool();
    sqlx::query("select pg_advisory_lock($1)")
        .bind(persistence::MIGRATION_LOCK_ID)
        .execute(&holder).await.unwrap();

    let started = std::time::Instant::now();
    let out = common::run_fau_migrate_with(&db, &[("MIGRATION_LOCK_WAIT_MS", "500")]).await;
    let elapsed = started.elapsed();

    assert!(!out.status.success(), "must fail, not hang");
    assert!(elapsed < std::time::Duration::from_secs(20), "took {elapsed:?}");
}

#[tokio::test]
async fn serve_performs_no_ddl() {
    // Spec section 3. The runtime role has no DDL rights (Task 5), so this is
    // belt and braces: an empty database must stay empty when serve is started.
    let db = TestDb::fresh().await;
    common::apply_roles(&db).await;
    let _ = common::start_serve_expecting_failure(&db).await;

    let tables: i64 = sqlx::query_scalar(
        "select count(*) from information_schema.tables where table_schema = 'public'")
        .fetch_one(&db.admin_pool()).await.unwrap();
    assert_eq!(tables, 0, "serve created schema objects");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test migrations`
Expected: FAIL.

- [ ] **Step 3: Write migration 0001**

`backend/migrations/0001_schema_contract.sql` — verbatim from spec §5, nothing else:

```sql
-- 0001: the schema contract. Creates the contract table and nothing else, so the
-- migration runner is proven end to end before any domain structure depends on it.
create table schema_contract (
  version    integer     primary key,
  applied_at timestamptz not null default now()
);

insert into schema_contract (version) values (1);
```

- [ ] **Step 4: Implement the runner**

`MIGRATION_LOCK_ID` is a fixed `i64` shared by every FAU migration process. Derive it from a
constant string at build time rather than typing a magic number, and export it so tests can contend
on the same key:

```rust
/// Outer lock for the whole migration run. PostgreSQL advisory locks are NOT
/// bounded by lock_timeout, so sqlx's own pg_advisory_lock would block forever.
/// We gate on this one first, with a deadline we control.
pub const MIGRATION_LOCK_ID: i64 = 0x4641_5530_0000_0001; // "FAU0" + sequence

async fn acquire_outer_lock(conn: &mut PgConnection, wait: Duration)
    -> Result<(), MigrateError>
{
    let deadline = Instant::now() + wait;
    loop {
        let got: bool = sqlx::query_scalar("select pg_try_advisory_lock($1)")
            .bind(MIGRATION_LOCK_ID).fetch_one(&mut *conn).await?;
        if got { return Ok(()); }
        if Instant::now() >= deadline { return Err(MigrateError::LockUnavailable); }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

The connection is built with `lock_timeout` set through connection options, which bounds the DDL
*inside* each migration:

```rust
let opts: PgConnectOptions = settings.url.parse()?;
let opts = opts.options([("lock_timeout", &settings.lock_timeout_ms.to_string())]);
```

Then `sqlx::migrate!("./migrations").run(&mut conn)`. Release the outer lock in all paths — the
session ends with the connection, so a crashed process releases it automatically, but release it
explicitly on the success path so a pooled connection is not left holding it.

sqlx's `migrate!` macro embeds the SQL at compile time, which is what lets the runtime image carry
the binary alone; `migrations/` is still copied into the image so the files can be inspected in a
running container, per spec §13.

Defaults: `MIGRATION_LOCK_TIMEOUT_MS=10000`, `MIGRATION_LOCK_WAIT_MS=30000`.

- [ ] **Step 5: Run the tests**

Run: `cd backend && cargo test --test migrations`
Expected: PASS.

- [ ] **Step 6: Leave in the working tree**

---

### Task 5: Database roles and the privilege split

**Files:**
- Create: `backend/db/roles.sql` (replaces the Task 3 stub)
- Modify: `backend/tests/common/mod.rs` (`apply_roles`)
- Test: `backend/tests/schema_review.rs` (first half)

**Interfaces:**
- Produces: roles `fau_migrate` (DDL owner) and `fau_app` (runtime, no DDL). Names are a fixed
  project contract — migration `0002` grants to `fau_app` by name, so #3424 must provision these
  exact names in production.

Role *creation* lives here rather than in a migration because spec §5 fixes `0001`'s content
exactly ("creates that table and nothing else") and roles are cluster-level objects, not schema.
Passwords never appear in `roles.sql`: it creates `fau_app` with `nologin` and no password, and the
deployment grants `login` with a password out of band. Compose does that in its init script.

- [ ] **Step 1: Write the failing test**

```rust
// backend/tests/schema_review.rs
mod common;
use common::TestDb;

#[tokio::test]
async fn runtime_role_cannot_perform_ddl() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await; // connects as fau_app

    let err = sqlx::query("create table sneaky (id int)").execute(&pool).await;
    assert!(err.is_err(), "runtime role must not have DDL rights");
}

#[tokio::test]
async fn runtime_role_can_read_and_write_the_spine() {
    let db = TestDb::migrated().await;
    let pool = db.app_pool().await;
    let n: i64 = sqlx::query_scalar("select count(*) from accounts")
        .fetch_one(&pool).await.expect("runtime role must read accounts");
    assert_eq!(n, 0);
}

#[tokio::test]
async fn runtime_role_is_not_a_superuser_and_cannot_bypass_rls() {
    let db = TestDb::migrated().await;
    let row: (bool, bool) = sqlx::query_as(
        "select rolsuper, rolbypassrls from pg_roles where rolname = 'fau_app'")
        .fetch_one(&db.admin_pool()).await.unwrap();
    assert_eq!(row, (false, false),
        "row-level security lands in #3418 and must not already be bypassed");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test schema_review`
Expected: FAIL.

- [ ] **Step 3: Write `backend/db/roles.sql`**

```sql
-- Database roles for FAU. Cluster-level objects, so not a migration.
-- Applied by the Compose init script and by the integration-test harness;
-- #3424 provisions the same two names in production.
--
-- No passwords here. roles.sql is committed; credentials are granted out of band.

do $$
begin
  if not exists (select 1 from pg_roles where rolname = 'fau_migrate') then
    create role fau_migrate nologin;
  end if;
  if not exists (select 1 from pg_roles where rolname = 'fau_app') then
    create role fau_app nologin;
  end if;
end
$$;

-- The migration role owns DDL. The runtime role gets connect and usage only;
-- table privileges are granted per table by the migration that creates it, so a
-- new table is inaccessible to the runtime until someone decides what it may do.
grant create, connect on database current_database() to fau_migrate;
grant connect on database current_database() to fau_app;
grant usage on schema public to fau_app;

-- Explicitly withhold the default. Without this, PUBLIC can create objects in
-- the public schema on PostgreSQL versions before 15 and, more importantly, the
-- intent is recorded where a reviewer will read it.
revoke create on schema public from public;
revoke create on schema public from fau_app;
```

`grant … on database current_database()` is not valid SQL — `current_database()` cannot appear
there. Generate the statement instead:

```sql
do $$
begin
  execute format('grant create, connect on database %I to fau_migrate', current_database());
  execute format('grant connect on database %I to fau_app', current_database());
end
$$;
```

- [ ] **Step 4: Extend the harness**

`common::apply_roles(&db)` runs `roles.sql` as the superuser, then grants login with a test
password: `alter role fau_app login password 'fau_app'` and the same for `fau_migrate`. Add
`TestDb::app_pool()` and make `TestDb::migration_url()` return the `fau_migrate` DSN.
`TestDb::migrated()` applies roles before migrations, since `0002` grants to `fau_app`.

- [ ] **Step 5: Run the tests**

Run: `cd backend && cargo test --test schema_review`
Expected: the three role tests PASS (`runtime_role_can_read_and_write_the_spine` still fails until
Task 6 creates `accounts`; that is expected and is the next task's entry condition).

- [ ] **Step 6: Leave in the working tree**

---

### Task 6: Migration 0002 — the identity and tenancy spine

**Files:**
- Create: `backend/migrations/0002_identity_and_tenancy.sql`
- Test: `backend/tests/schema_spine.rs`, `backend/tests/schema_review.rs` (second half)

**Interfaces:**
- Produces: tables `accounts`, `identity_mappings`, `tenants`, `memberships`, `roles`,
  `role_assignments`, and `schema_contract` row `(2)`.

- [ ] **Step 1: Write the failing tests**

```rust
// backend/tests/schema_spine.rs
mod common;
use common::TestDb;
use uuid::Uuid;

async fn seed_two_tenants(db: &TestDb) -> (Uuid, Uuid, Uuid) {
    let pool = db.admin_pool();
    let (t1, t2, acc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    sqlx::query("insert into tenants (id, name, status) values ($1,'FAU A','active'), ($2,'FAU B','active')")
        .bind(t1).bind(t2).execute(&pool).await.unwrap();
    sqlx::query("insert into accounts (id, email) values ($1, 'a@example.test')")
        .bind(acc).execute(&pool).await.unwrap();
    (t1, t2, acc)
}

#[tokio::test]
async fn cross_tenant_reference_is_rejected_by_the_database() {
    // Spec section 5, property 1 — #3412's central integrity rule.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, t2, acc) = seed_two_tenants(&db).await;

    let m1 = Uuid::now_v7();
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1).bind(m1).bind(acc).execute(&pool).await.unwrap();

    let r2 = Uuid::now_v7();
    sqlx::query("insert into roles (tenant_id, id, name, capability_class) values ($1,$2,'Leder','admin')")
        .bind(t2).bind(r2).execute(&pool).await.unwrap();

    // Tenant B's role assigned to tenant A's membership, written directly in SQL.
    let err = sqlx::query(
        "insert into role_assignments
           (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
         values ($1, $2, $3, $4, date '2026-08-01', date '2027-08-01')")
        .bind(t1).bind(Uuid::now_v7()).bind(m1).bind(r2)
        .execute(&pool).await;

    assert!(err.is_err(), "composite foreign key did not reject a cross-tenant reference");
}

#[tokio::test]
async fn role_period_must_be_a_non_empty_half_open_interval() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, _t2, acc) = seed_two_tenants(&db).await;
    let (m, r) = (Uuid::now_v7(), Uuid::now_v7());
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1).bind(m).bind(acc).execute(&pool).await.unwrap();
    sqlx::query("insert into roles (tenant_id, id, name, capability_class) values ($1,$2,'Medlem','member')")
        .bind(t1).bind(r).execute(&pool).await.unwrap();

    for (starts, ends) in [("2027-08-01", "2026-08-01"), ("2026-08-01", "2026-08-01")] {
        let err = sqlx::query(
            "insert into role_assignments
               (tenant_id, id, membership_id, role_id, starts_on, ends_on_exclusive)
             values ($1,$2,$3,$4,$5::date,$6::date)")
            .bind(t1).bind(Uuid::now_v7()).bind(m).bind(r).bind(starts).bind(ends)
            .execute(&pool).await;
        assert!(err.is_err(), "accepted {starts}..{ends}");
    }
}

#[tokio::test]
async fn ends_on_exclusive_is_mandatory() {
    let db = TestDb::migrated().await;
    let null_allowed: bool = sqlx::query_scalar(
        "select is_nullable = 'YES' from information_schema.columns
         where table_name = 'role_assignments' and column_name = 'ends_on_exclusive'")
        .fetch_one(&db.admin_pool()).await.unwrap();
    assert!(!null_allowed, "an open-ended role period must be impossible");
}

#[tokio::test]
async fn an_account_may_hold_several_identity_mappings() {
    // ADR-003 decision 3: the rule most expensive to add later.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let acc = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email) values ($1,'m@example.test')")
        .bind(acc).execute(&pool).await.unwrap();

    for issuer in ["https://a.hanko.example", "https://b.other.example"] {
        sqlx::query("insert into identity_mappings (issuer, subject, account_id) values ($1,'sub-1',$2)")
            .bind(issuer).bind(acc).execute(&pool).await
            .expect("two issuers must map to one account");
    }
}

#[tokio::test]
async fn one_membership_per_account_per_tenant() {
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let (t1, _t2, acc) = seed_two_tenants(&db).await;
    sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1).bind(Uuid::now_v7()).bind(acc).execute(&pool).await.unwrap();
    let dup = sqlx::query("insert into memberships (tenant_id, id, account_id) values ($1,$2,$3)")
        .bind(t1).bind(Uuid::now_v7()).bind(acc).execute(&pool).await;
    assert!(dup.is_err(), "duplicate membership accepted");
}

#[tokio::test]
async fn locale_columns_exist_with_the_decided_defaults() {
    // #3439. Nothing may hard-code two locales; these are free-text BCP 47 tags.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();

    let account_locale_nullable: bool = sqlx::query_scalar(
        "select is_nullable = 'YES' from information_schema.columns
         where table_name = 'accounts' and column_name = 'locale'")
        .fetch_one(&pool).await.unwrap();
    assert!(account_locale_nullable);

    let t = Uuid::now_v7();
    sqlx::query("insert into tenants (id, name, status) values ($1,'FAU','pending')")
        .bind(t).execute(&pool).await.unwrap();
    let default_locale: String = sqlx::query_scalar(
        "select default_locale from tenants where id = $1")
        .bind(t).fetch_one(&pool).await.unwrap();
    assert_eq!(default_locale, "nb-NO");
}

#[tokio::test]
async fn tenant_status_is_constrained() {
    let db = TestDb::migrated().await;
    let bad = sqlx::query("insert into tenants (id, name, status) values ($1,'X','deleted')")
        .bind(Uuid::now_v7()).execute(&db.admin_pool()).await;
    assert!(bad.is_err(), "status must be pending/active/closed");
}

#[tokio::test]
async fn retention_months_defaults_to_three() {
    // ADR-003 decision 6a: member-elected retention is designed for, not built.
    let db = TestDb::migrated().await;
    let pool = db.admin_pool();
    let acc = Uuid::now_v7();
    sqlx::query("insert into accounts (id, email) values ($1,'r@example.test')")
        .bind(acc).execute(&pool).await.unwrap();
    let months: i32 = sqlx::query_scalar("select retention_months from accounts where id = $1")
        .bind(acc).fetch_one(&pool).await.unwrap();
    assert_eq!(months, 3);
}
```

And the schema review test, which is a standing guard rather than a test of one migration:

```rust
// backend/tests/schema_review.rs (continued)
#[tokio::test]
async fn no_application_table_contains_key_material() {
    // Spec section 5 and ADR-003 decision 5: keys live only in the key service.
    // A wrapped key on a row is a defect, and this is the test that says so.
    let db = TestDb::migrated().await;
    let columns: Vec<(String, String)> = sqlx::query_as(
        "select table_name, column_name from information_schema.columns
         where table_schema = 'public'")
        .fetch_all(&db.admin_pool()).await.unwrap();

    const FORBIDDEN: &[&str] = &[
        "key", "dek", "kek", "secret", "private", "passphrase",
        "password", "cipher", "nonce", "wrapped",
    ];
    let mut offenders = Vec::new();
    for (table, column) in &columns {
        let c = column.to_ascii_lowercase();
        for needle in FORBIDDEN {
            // Substring, not equality: key_id, wrapped_dek and encryption_key all fail.
            if c.contains(needle) {
                offenders.push(format!("{table}.{column}"));
            }
        }
    }
    assert!(offenders.is_empty(),
        "possible key material in application tables: {offenders:?}. \
         Keys belong in the key service (ADR-003 decision 5). If a column name is a \
         false positive, rename it — this test is deliberately blunt.");
}

#[tokio::test]
async fn the_providers_subject_appears_only_in_identity_mappings() {
    // ADR-003 decision 3.
    let db = TestDb::migrated().await;
    let tables: Vec<String> = sqlx::query_scalar(
        "select distinct table_name from information_schema.columns
         where table_schema = 'public' and column_name in ('subject', 'sub', 'issuer')")
        .fetch_all(&db.admin_pool()).await.unwrap();
    assert_eq!(tables, vec!["identity_mappings".to_string()]);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test schema_spine --test schema_review`
Expected: FAIL — relations do not exist.

- [ ] **Step 3: Write migration 0002**

```sql
-- 0002: the identity and tenancy spine. Reconciles the approved #3412 model with
-- ADR-003 (identity and encryption) and #3439 (localisation).
--
-- Three properties are load-bearing and must not be softened later:
--   1. Every reference between tenant data uses the composite key (tenant_id, id).
--   2. An account may hold several identity mappings at once.
--   3. Role periods are half-open [starts_on, ends_on_exclusive) with a mandatory end.
--
-- No key material of any kind appears in any table. Document and tenant keys live
-- only in the key service (ADR-003 decision 5).

-- Global. Accounts and authentication are not tenant data.
create table accounts (
  id               uuid        primary key,
  email            text        not null unique,
  verified_at      timestamptz,
  disabled_at      timestamptz,
  -- #3439: a BCP 47 tag, nullable because "no preference" is distinct from nb-NO.
  -- Deliberately unconstrained: nothing may hard-code two locales.
  locale           text,
  -- ADR-003 decision 6a. The field ships from the first migration; the screen does not.
  retention_months integer     not null default 3 check (retention_months > 0),
  created_at       timestamptz not null default now()
);

-- ADR-003 decision 3. (issuer, subject) is the key; account_id is NOT unique, which
-- is what allows two providers to run concurrently during a migration.
create table identity_mappings (
  issuer     text        not null,
  subject    text        not null,
  account_id uuid        not null references accounts (id) on delete cascade,
  created_at timestamptz not null default now(),
  primary key (issuer, subject)
);
create index identity_mappings_account_id_idx on identity_mappings (account_id);

create table tenants (
  id             uuid        primary key,
  name           text        not null,
  status         text        not null check (status in ('pending', 'active', 'closed')),
  school_id      uuid,
  -- #3439: what this FAU uses for anyone without a personal preference.
  default_locale text        not null default 'nb-NO',
  created_at     timestamptz not null default now()
);

-- From here down every table carries tenant_id and every reference is composite.
create table memberships (
  tenant_id  uuid        not null references tenants (id),
  id         uuid        not null,
  account_id uuid        not null references accounts (id),
  revoked_at timestamptz,
  created_at timestamptz not null default now(),
  primary key (tenant_id, id),
  -- #3412: one membership per account per FAU.
  unique (tenant_id, account_id)
);

create table roles (
  tenant_id        uuid not null references tenants (id),
  id               uuid not null,
  name             text not null,
  -- #3412: the privilege follows the explicit class, never the free-text name.
  capability_class text not null check (capability_class in ('member', 'admin')),
  unit_id          uuid,
  cohort_id        uuid,
  created_at       timestamptz not null default now(),
  primary key (tenant_id, id)
);

create table role_assignments (
  tenant_id         uuid not null references tenants (id),
  id                uuid not null,
  membership_id     uuid not null,
  role_id           uuid not null,
  -- Local calendar dates in Europe/Oslo. The server's clock decides access.
  starts_on          date not null,
  ends_on_exclusive  date not null,
  revoked_at        timestamptz,
  granted_by        uuid,
  created_at        timestamptz not null default now(),
  primary key (tenant_id, id),
  -- The composite foreign keys are the isolation guarantee that ships now,
  -- ahead of row-level security in #3418.
  foreign key (tenant_id, membership_id) references memberships (tenant_id, id),
  foreign key (tenant_id, role_id)       references roles (tenant_id, id),
  foreign key (tenant_id, granted_by)    references memberships (tenant_id, id),
  constraint role_period_is_non_empty check (starts_on < ends_on_exclusive)
);
create index role_assignments_membership_idx on role_assignments (tenant_id, membership_id);

-- The runtime role gets exactly what it needs on exactly these tables. A future
-- table is unreachable from the application until a migration says otherwise, and
-- finalised history and audit (#3419, #3421) will be granted select and insert only.
grant select, insert, update, delete on
  accounts, identity_mappings, tenants, memberships, roles, role_assignments
  to fau_app;
grant select on schema_contract to fau_app;

insert into schema_contract (version) values (2);
```

Note `granted_by` references `memberships`, so an administrator who granted a role is recorded as a
membership in the same tenant — consistent with #3412, where the actor is always tenant-scoped.

- [ ] **Step 4: Run the tests**

Run: `cd backend && cargo test --test schema_spine --test schema_review`
Expected: PASS, including the two Task 5 tests that were waiting on `accounts`.

- [ ] **Step 5: Leave in the working tree**

---

### Task 7: The schema contract gate

**Files:**
- Create: `backend/crates/domain/src/schema_contract.rs`
- Create: `backend/crates/persistence/src/contract.rs`
- Modify: `backend/crates/app/src/main.rs`
- Test: `backend/tests/contract_gate.rs`, unit tests in `domain/src/schema_contract.rs`

**Interfaces:**
- Produces:
  - `domain`: `pub const MINIMUM_CONTRACT_VERSION: i32 = 2;`
    `pub fn is_compatible(found: i32) -> bool { found >= MINIMUM_CONTRACT_VERSION }`
  - `persistence`: `pub async fn read_contract_version(pool: &PgPool) -> Result<i32, sqlx::Error>`
    — `select coalesce(max(version), 0) from schema_contract`.

- [ ] **Step 1: Write the failing tests**

```rust
// backend/crates/domain/src/schema_contract.rs (unit tests at the bottom)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lower_contract_is_incompatible() {
        assert!(!is_compatible(MINIMUM_CONTRACT_VERSION - 1));
    }

    #[test]
    fn an_equal_contract_is_compatible() {
        assert!(is_compatible(MINIMUM_CONTRACT_VERSION));
    }

    #[test]
    fn a_higher_contract_is_compatible() {
        // ADR-001 expand/contract: an additive migration rolls out before the new
        // pods do, so a running binary must tolerate a database ahead of it.
        assert!(is_compatible(MINIMUM_CONTRACT_VERSION + 1));
    }
}
```

```rust
// backend/tests/contract_gate.rs
mod common;
use common::TestDb;

#[tokio::test]
async fn refuses_to_serve_below_the_minimum() {
    let db = TestDb::migrated().await;
    sqlx::query("delete from schema_contract where version = 2")
        .execute(&db.admin_pool()).await.unwrap();

    let out = common::run_fau_serve_until_exit(&db).await;
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("schema contract"), "message was: {stderr}");
}

#[tokio::test]
async fn serves_above_the_minimum() {
    let db = TestDb::migrated().await;
    sqlx::query("insert into schema_contract (version) values (99)")
        .execute(&db.admin_pool()).await.unwrap();

    let app = common::spawn_serve(&db).await;
    let status = app.get("/health/live").await.status();
    assert_eq!(status, 200, "a database ahead of the binary must still be served");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test -p fau-domain schema_contract && cargo test --test contract_gate`
Expected: FAIL.

- [ ] **Step 3: Implement**

The gate runs once at startup, after the pool is built. A database that is *unreachable* at startup
is not a contract failure — spec §4 says the process stays up with readiness false — so the startup
gate applies only when the query succeeds and returns a version below the minimum. If the query
fails to connect, log a warning, leave readiness false, and let the readiness loop retry.

- [ ] **Step 4: Run the tests**

Run: `cd backend && cargo test -p fau-domain && cargo test --test contract_gate`
Expected: PASS.

- [ ] **Step 5: Leave in the working tree**

---

### Task 8: HTTP surface — liveness, placeholder, error contract, routing

**Files:**
- Create: `backend/crates/app/src/http/mod.rs`, `http/health.rs`, `http/error.rs`,
  `http/placeholder.rs`, `http/placeholder.html`
- Modify: `backend/crates/app/src/main.rs`
- Test: `backend/tests/http_surface.rs`

**Interfaces:**
- Consumes: `ServeConfig` (Task 2), `ReadinessState` (Task 9 — stub it as always-ready here and
  replace in Task 9).
- Produces:
  - `pub fn router(state: AppState) -> axum::Router`
  - `pub struct ApiError { code: ErrorCode, params: BTreeMap<String, ParamValue>, status: StatusCode }`
    serialising as `{"code": "...", "params": {...}, "request_id": "..."}`.
  - `pub struct AppState { pub readiness: ReadinessState, pub version: &'static str }`

- [ ] **Step 1: Write the failing tests**

```rust
// backend/tests/http_surface.rs
mod common;
use common::TestDb;

#[tokio::test]
async fn liveness_is_200_and_touches_nothing() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/health/live").await;
    assert_eq!(res.status(), 200);
}

#[tokio::test]
async fn liveness_stays_200_when_the_database_is_unreachable() {
    // Spec section 9: a dependency outage must not cause a restart loop.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    db.sever_connections().await; // terminates backends and revokes connect
    assert_eq!(app.get("/health/live").await.status(), 200);
}

#[tokio::test]
async fn root_serves_the_bokmal_placeholder_as_html() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/").await;
    assert_eq!(res.status(), 200);
    assert!(res.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    assert!(res.text().await.contains("lang=\"nb-NO\""));
}

#[tokio::test]
async fn unknown_api_routes_are_404_and_never_html() {
    // Spec section 8 — the rule that keeps a client from parsing a login page as JSON.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    for path in ["/api/v1/nope", "/api/v1/", "/api/v1/tenants/1"] {
        let res = app.get(path).await;
        assert_eq!(res.status(), 404, "{path}");
        let ct = res.headers()["content-type"].to_str().unwrap().to_string();
        assert!(ct.starts_with("application/json"), "{path} returned {ct}");
        let body = res.text().await;
        assert!(!body.contains("<html"), "{path} fell back to HTML");
    }
}

#[tokio::test]
async fn missing_assets_are_404_and_never_html() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/assets/does-not-exist.js").await;
    assert_eq!(res.status(), 404);
    assert!(!res.text().await.contains("<html"));
}

#[tokio::test]
async fn api_errors_carry_a_stable_code_and_request_id_but_no_display_text() {
    // Spec section 11 and #3439: the client renders the sentence.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/api/v1/nope").await;
    let body: serde_json::Value = res.json().await;

    assert_eq!(body["code"], "not_found");
    assert!(body["request_id"].is_string());
    assert!(body.get("message").is_none(), "error carried display text: {body}");
    assert!(body.get("detail").is_none(), "error carried display text: {body}");

    // No Norwegian prose anywhere in the payload.
    let raw = body.to_string();
    for word in ["ikke", "finnes", "feil", "Ugyldig"] {
        assert!(!raw.contains(word), "localisable prose in error body: {raw}");
    }
}

#[tokio::test]
async fn reserved_prefixes_exist_but_carry_no_routes() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    // /app/ is reserved for #3422 and returns the placeholder shell, not 404,
    // so the client-route fallback contract is in place from the start.
    assert_eq!(app.get("/app/anything").await.status(), 200);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test http_surface`
Expected: FAIL.

- [ ] **Step 3: Implement the router**

Order matters. Register `/api/v1/` and `/assets/` with explicit JSON-404 fallbacks *before* the
`/app/` HTML fallback, so a missing asset can never resolve to HTML:

```rust
Router::new()
    .route("/health/live", get(health::live))
    .route("/health/ready", get(health::ready))
    .route("/", get(placeholder::page))
    .nest("/api/v1", Router::new().fallback(error::json_not_found))
    .nest("/assets", Router::new().fallback(error::json_not_found))
    .nest("/app",   Router::new().fallback(placeholder::page))
    .fallback(error::json_not_found)
```

The top-level fallback is JSON rather than HTML on purpose: an unknown top-level path is more
likely a mistyped API call than a browser navigation, and ADR-001 forbids HTML fallback outside
`/app/`.

`ErrorCode` is a `#[non_exhaustive]` enum in `domain` serialising to `snake_case` strings. `ApiError`
serialises `{code, params, request_id}` and nothing else — no `Display` impl is exposed to the
response body. `params` values are a bounded `ParamValue` enum (`Int`, `Str`, `Bool`) so an error
cannot smuggle an arbitrary payload.

`placeholder.html`: Bokmål, `<html lang="nb-NO">`, no JavaScript, no external references, no
configuration values. `include_str!`, served with `content-type: text/html; charset=utf-8` and
`cache-control: no-cache` (ADR-001: HTML revalidates).

- [ ] **Step 4: Run the tests**

Run: `cd backend && cargo test --test http_surface`
Expected: PASS.

- [ ] **Step 5: Leave in the working tree**

---

### Task 9: Readiness

**Files:**
- Create: `backend/crates/app/src/readiness.rs`, `backend/crates/persistence/src/health.rs`
- Modify: `backend/crates/app/src/http/health.rs`
- Test: `backend/tests/readiness.rs`

**Interfaces:**
- Consumes: `read_contract_version` (Task 7), router (Task 8).
- Produces:
  - `pub struct ReadinessState(Arc<...>)` — `Clone`, with `set_initialised()`,
    `begin_shutdown()`, and `async fn probe(&self, pool: &PgPool) -> Readiness`.
  - `pub enum Readiness { Ready, NotReady(NotReadyReason) }` where
    `NotReadyReason` is `Initialising | Database | SchemaContract { found: i32, minimum: i32 } | ShuttingDown`.
  - `persistence::db_check(pool) -> Result<(), DbCheckError>` — `select 1` under a **one-second**
    timeout.
- Cache: a successful probe is reused for 1 000 ms; a failing probe is **not** cached, so recovery
  is visible on the next scrape rather than up to a second later.

- [ ] **Step 1: Write the failing tests**

```rust
// backend/tests/readiness.rs
mod common;
use common::TestDb;

#[tokio::test]
async fn database_down_then_back_does_not_restart_the_process() {
    // Spec section 15, the third ADR-001 evidence row.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    assert_eq!(app.get("/health/ready").await.status(), 200);

    db.sever_connections().await;
    let ready = common::poll_until(|| app.get("/health/ready"), |r| r.status() == 503,
        std::time::Duration::from_secs(10)).await;
    assert!(ready, "readiness never went false");
    assert_eq!(app.get("/health/live").await.status(), 200, "liveness must not follow");
    assert!(app.is_running(), "the process restarted or exited");

    db.restore_connections().await;
    let back = common::poll_until(|| app.get("/health/ready"), |r| r.status() == 200,
        std::time::Duration::from_secs(15)).await;
    assert!(back, "readiness never recovered");
    assert!(app.is_running());
}

#[tokio::test]
async fn readiness_is_503_when_the_contract_is_below_the_minimum() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    sqlx::query("delete from schema_contract where version = 2")
        .execute(&db.admin_pool()).await.unwrap();
    let dropped = common::poll_until(|| app.get("/health/ready"), |r| r.status() == 503,
        std::time::Duration::from_secs(5)).await;
    assert!(dropped);
}

#[tokio::test]
async fn readiness_body_names_no_internal_address() {
    // ADR-001: minimal responses without internal addresses.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let body = app.get("/health/ready").await.text().await;
    for leak in ["postgres://", "127.0.0.1", "5432", "password"] {
        assert!(!body.contains(leak), "readiness leaked {leak}: {body}");
    }
}

#[tokio::test]
async fn a_failing_probe_is_not_cached() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    db.sever_connections().await;
    common::poll_until(|| app.get("/health/ready"), |r| r.status() == 503,
        std::time::Duration::from_secs(10)).await;
    db.restore_connections().await;
    // Recovery must be visible well inside the cache window of a *successful* probe.
    let back = common::poll_until(|| app.get("/health/ready"), |r| r.status() == 200,
        std::time::Duration::from_secs(5)).await;
    assert!(back);
}
```

`TestDb::sever_connections` does `alter database … connection limit 0`, `revoke connect … from
fau_app`, then `pg_terminate_backend` for every session on that database — a repeatable outage
without stopping the shared container. `restore_connections` reverses it.

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test readiness`
Expected: FAIL.

- [ ] **Step 3: Implement**

`db_check` uses `tokio::time::timeout(Duration::from_secs(1), sqlx::query("select 1").execute(pool))`.
The pool is configured with `acquire_timeout` shorter than that (900 ms) so a saturated pool surfaces
as not-ready rather than as a timeout ambiguity.

The bounded retry the spec asks for is the probe itself: readiness is evaluated per scrape, not by a
background reconnect loop, so there is no state to get stuck. `sqlx`'s pool reconnects on demand.

- [ ] **Step 4: Run the tests**

Run: `cd backend && cargo test --test readiness`
Expected: PASS.

- [ ] **Step 5: Leave in the working tree**

---

### Task 10: Structured logging and request context

**Files:**
- Create: `backend/crates/app/src/telemetry.rs`, `backend/crates/app/src/http/request_context.rs`
- Modify: `backend/crates/app/src/http/mod.rs`, `main.rs`
- Test: `backend/tests/logging.rs`

**Interfaces:**
- Produces:
  - `pub fn init(log_level: &str, service_version: &'static str)` — installs the JSON subscriber.
  - `pub struct RequestId(Uuid)` — extension set by the middleware, echoed in the
    `x-request-id` response header and in every error body.
  - Access log line per request with fields: `timestamp`, `level`, `service_version`, `request_id`,
    `route` (the `MatchedPath` template), `method`, `status`, `duration_ms`.

- [ ] **Step 1: Write the failing tests**

```rust
// backend/tests/logging.rs
mod common;
use common::TestDb;

#[tokio::test]
async fn every_line_is_json_with_a_conventional_level() {
    // Spec section 10: Alloy's stage.json extracts a top-level `level` and promotes
    // it to a Loki label. A non-JSON line or a renamed field breaks the dashboards.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    app.get("/health/live").await;
    let lines = app.captured_stdout().await;

    assert!(!lines.is_empty(), "no log output captured");
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("not JSON: {line} ({e})"));
        let level = v["level"].as_str().expect("no top-level level field");
        assert!(["TRACE","DEBUG","INFO","WARN","ERROR"].contains(&level), "level was {level}");
    }
}

#[tokio::test]
async fn the_access_log_uses_the_route_template_not_the_uri() {
    // Spec section 10: the template is what keeps identifiers out of the logs.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    app.get("/api/v1/tenants/0190c3f2-dead-7000-8000-000000000001").await;
    let lines = app.captured_stdout().await;
    let joined = lines.join("\n");
    assert!(!joined.contains("0190c3f2-dead-7000-8000-000000000001"),
        "a path identifier reached the log: {joined}");
}

#[tokio::test]
async fn sentinel_secrets_never_appear_in_log_output() {
    // Spec section 15. The sentinels are planted in exactly the places a leak
    // historically comes from: the DSN password, a query string, a header, a cookie.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_sentinels(&db).await;

    app.get("/?token=SENTINEL_QUERY").await;
    app.get_with_headers("/health/live", &[
        ("authorization", "Bearer SENTINEL_AUTHZ"),
        ("cookie", "session=SENTINEL_COOKIE"),
        ("x-email", "SENTINEL_EMAIL@example.test"),
    ]).await;
    app.get("/api/v1/nope?q=SENTINEL_QUERY2").await;

    let captured = app.captured_stdout().await.join("\n");
    for sentinel in [
        "SENTINEL_DB_PASSWORD",   // planted inside DATABASE_URL
        "SENTINEL_QUERY", "SENTINEL_QUERY2",
        "SENTINEL_AUTHZ", "SENTINEL_COOKIE", "SENTINEL_EMAIL",
    ] {
        assert!(!captured.contains(sentinel),
            "{sentinel} reached the log output:\n{captured}");
    }
}

#[tokio::test]
async fn the_request_id_is_echoed_and_logged() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let res = app.get("/health/live").await;
    let id = res.headers()["x-request-id"].to_str().unwrap().to_string();
    assert!(!id.is_empty());
    let captured = app.captured_stdout().await.join("\n");
    assert!(captured.contains(&id), "request id {id} not in the log");
}

#[tokio::test]
async fn no_log_is_exported_over_otlp() {
    // Spec section 10: Alloy has an OTLP receiver, so exporting logs there too
    // would double-collect every line. OTLP is reserved for traces.
    let manifest = include_str!("../crates/app/Cargo.toml");
    assert!(!manifest.contains("opentelemetry-appender"),
        "an OTLP log appender was added; spec section 10 forbids it");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test logging`
Expected: FAIL.

- [ ] **Step 3: Implement**

`telemetry::init` builds `tracing_subscriber::fmt().json().with_current_span(true)
.with_span_list(false).with_target(false)` plus an `EnvFilter` from `LOG_LEVEL`. `flatten_event(true)`
so `level` stays top-level where Alloy's `stage.json` finds it.

The access log is written by our own middleware rather than `tower_http::trace::TraceLayer`, because
`TraceLayer`'s default `on_request` logs `http.uri` — the whole URI, query string included. That is
exactly the leak the sentinel test catches. Our middleware records only:

```rust
let route = req.extensions().get::<MatchedPath>()
    .map(|m| m.as_str().to_owned())
    .unwrap_or_else(|| "<unmatched>".to_owned());
let method = req.method().clone();
// Deliberately absent: uri, query, headers, cookies.
```

`spawn_serve_with_sentinels` plants `SENTINEL_DB_PASSWORD` as the password inside `DATABASE_URL`, so
the test also proves that a connection error message — the classic way a DSN reaches a log — does not
carry it. `MigrateError` and `DbCheckError` therefore must not include the sqlx error's `Display`
where it can contain a DSN; map connection errors to a fixed message plus the error *kind*.

- [ ] **Step 4: Run the tests**

Run: `cd backend && cargo test --test logging`
Expected: PASS.

- [ ] **Step 5: Leave in the working tree**

---

### Task 11: Graceful shutdown

**Files:**
- Create: `backend/crates/app/src/shutdown.rs`
- Modify: `backend/crates/app/src/main.rs`
- Test: `backend/tests/shutdown.rs`

**Interfaces:**
- Produces: `pub async fn signal(readiness: ReadinessState) -> ()` — resolves on SIGTERM or SIGINT,
  sets readiness false, then returns. Used as axum's `with_graceful_shutdown` future.
- Drain bound: **25 seconds**, then the process exits regardless.

- [ ] **Step 1: Write the failing tests**

```rust
// backend/tests/shutdown.rs
mod common;
use common::TestDb;
use std::time::{Duration, Instant};

#[tokio::test]
async fn sigterm_sets_readiness_false_before_the_listener_closes() {
    // Spec section 12: stop taking traffic first, then drain. If the listener closed
    // first, the load balancer would still be sending requests to a closed socket.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    assert_eq!(app.get("/health/ready").await.status(), 200);

    app.send_sigterm();
    let saw_503 = common::poll_until(|| app.get("/health/ready"),
        |r| r.status() == 503, Duration::from_secs(3)).await;
    assert!(saw_503, "readiness did not go false while still serving");
}

#[tokio::test]
async fn an_in_flight_request_completes_after_sigterm() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_test_routes(&db).await;

    // /test/slow sleeps 2s; only compiled under cfg(feature = "test-routes").
    let pending = app.get_async("/test/slow");
    tokio::time::sleep(Duration::from_millis(200)).await;
    app.send_sigterm();

    let res = pending.await;
    assert_eq!(res.status(), 200, "an in-flight request was cut off");
}

#[tokio::test]
async fn the_process_exits_within_the_drain_bound() {
    let db = TestDb::migrated().await;
    let app = common::spawn_serve(&db).await;
    let started = Instant::now();
    app.send_sigterm();
    let code = app.wait().await;
    assert!(started.elapsed() < Duration::from_secs(30),
        "took {:?}, bound is 25s", started.elapsed());
    assert!(code.success(), "clean shutdown must exit 0, got {code:?}");
}

#[tokio::test]
async fn an_aborted_transaction_is_never_acknowledged() {
    // Spec section 12. /test/slow-write opens a transaction, sleeps past the drain
    // bound, then commits. The client must see a failure, not a 200, and the row
    // must not be present.
    let db = TestDb::migrated().await;
    let app = common::spawn_serve_with_test_routes(&db).await;
    let pending = app.get_async("/test/slow-write?delay_ms=40000");
    tokio::time::sleep(Duration::from_millis(300)).await;
    app.send_sigterm();

    let outcome = pending.await;
    let acknowledged = outcome.map(|r| r.status().is_success()).unwrap_or(false);
    let rows: i64 = sqlx::query_scalar("select count(*) from tenants where name = 'slow-write'")
        .fetch_one(&db.admin_pool()).await.unwrap();
    assert!(!(acknowledged && rows == 0), "success was acknowledged for a rolled-back write");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test shutdown`
Expected: FAIL.

- [ ] **Step 3: Implement**

`tokio::signal::unix::signal(SignalKind::terminate())` and `ctrl_c()`, selected on. On fire:
`readiness.begin_shutdown()`, log one line at INFO, sleep a short *pre-drain* (500 ms) so at least
one readiness scrape observes 503 before the listener closes, then return — axum stops accepting and
waits for in-flight requests.

Wrap the whole `axum::serve(...).with_graceful_shutdown(...)` in
`tokio::time::timeout(Duration::from_secs(25), ...)`. On timeout, log at WARN and exit anyway;
in-flight transactions are rolled back by PostgreSQL when their connections drop, which is what
makes "never acknowledged" true rather than merely intended.

The `test-routes` cargo feature is off by default and is never enabled in the image build. Add a
test asserting the release binary has no `/test/` route.

- [ ] **Step 4: Run the tests**

Run: `cd backend && cargo test --test shutdown --features test-routes`
Expected: PASS.

- [ ] **Step 5: Leave in the working tree**

---

### Task 12: Container image

**Files:**
- Create: `/workspace/Dockerfile`
- Modify: `/workspace/.dockerignore`
- Create: `backend/tests/image.rs` (ignored by default; run explicitly)

**Interfaces:**
- Produces: image tag `fau/app:dev` locally. Entry point `/app/fau`, exec form, default argument
  `serve`.

- [ ] **Step 1: Write the failing test**

```rust
// backend/tests/image.rs — run with: cargo test --test image -- --ignored
use std::process::Command;

fn docker(args: &[&str]) -> std::process::Output {
    Command::new("docker").args(args).output().expect("docker")
}

const IMAGE: &str = "fau/app:dev";

#[test]
#[ignore = "builds a container image; run explicitly"]
fn image_contract() {
    let build = Command::new("docker")
        .args(["build", "-t", IMAGE, "-f", "Dockerfile", "."])
        .current_dir("/workspace").status().expect("docker build");
    assert!(build.success());

    // Non-root.
    let out = docker(&["run", "--rm", IMAGE, "--version"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("fau "));

    let uid = docker(&["run", "--rm", "--entrypoint", "id", IMAGE, "-u"]);
    let uid = String::from_utf8_lossy(&uid.stdout).trim().to_string();
    assert_ne!(uid, "0", "image runs as root");

    // Read-only root filesystem with a tmpfs /tmp.
    let ro = docker(&["run", "--rm", "--read-only", "--tmpfs", "/tmp",
                      "--entrypoint", "sh", IMAGE, "-c", "touch /tmp/ok && ! touch /nope"]);
    assert!(ro.status.success(), "read-only rootfs contract failed");

    // The runtime stage carries the binary, CA certificates and migrations — no toolchain.
    for absent in ["/usr/local/cargo", "/usr/bin/gcc", "/usr/local/rustup"] {
        let probe = docker(&["run", "--rm", "--entrypoint", "sh", IMAGE, "-c",
                             &format!("test ! -e {absent}")]);
        assert!(probe.status.success(), "{absent} is present in the runtime stage");
    }
    let migrations = docker(&["run", "--rm", "--entrypoint", "sh", IMAGE, "-c",
                              "ls /app/migrations/0002_identity_and_tenancy.sql"]);
    assert!(migrations.status.success(), "migrations/ missing from the image");

    // No secret reached the build context.
    let env_file = docker(&["run", "--rm", "--entrypoint", "sh", IMAGE, "-c", "test ! -e /app/.env"]);
    assert!(env_file.status.success());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test image -- --ignored`
Expected: FAIL — no Dockerfile.

- [ ] **Step 3: Write the Dockerfile**

```dockerfile
# syntax=docker/dockerfile:1.7
# FAU application image. Dockerfile.agent is the agent-box image and is unrelated.
# Base images are pinned by digest: a moving tag would break the rule that one
# digest is used across test, migration and deploy.

FROM rust:1.98.1-bookworm@sha256:c49256cbe5ea0188bc658a689500d70c41eb51f009a7a7be209caf60a944f3ec AS build
WORKDIR /src
# Manifests first, so a source-only change does not re-resolve the dependency graph.
COPY backend/Cargo.toml backend/Cargo.lock backend/rust-toolchain.toml ./
COPY backend/crates ./crates
COPY backend/migrations ./migrations
ARG FAU_BUILD_REVISION=unknown
ENV FAU_BUILD_REVISION=${FAU_BUILD_REVISION}
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin fau && \
    cp /src/target/release/fau /fau

FROM debian:bookworm-slim@sha256:f3034a6ec3c1205360777c4aae76234998866ad18806ae62b63a3f84ccad782b AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin fau
WORKDIR /app
COPY --from=build /fau /app/fau
# Copied so the exact SQL can be read inside a running container. The binary
# embeds them at compile time; these files are evidence, not the source of truth.
COPY backend/migrations /app/migrations
USER 10001:10001
EXPOSE 8000
ENTRYPOINT ["/app/fau"]
CMD ["serve"]
```

- [ ] **Step 4: Re-include `backend/` in `.dockerignore`**

Append to `/workspace/.dockerignore`, keeping the deny-by-default shape:

```
# Application image (Dockerfile at the repository root). The context is the project
# root, which holds .env, so the exclude-everything rule above stays and only the
# paths the app build reads are added back.
!backend
backend/target
!Dockerfile
```

- [ ] **Step 5: Run the test**

Run: `cd backend && cargo test --test image -- --ignored`
Expected: PASS.

- [ ] **Step 6: Leave in the working tree**

---

### Task 13: Compose stack, acceptance evidence and operator documentation

**Files:**
- Create: `/workspace/compose.yaml`
- Create: `/workspace/backend/db/compose-init.sh`
- Modify: `/workspace/.env.example`
- Create: `/workspace/docs/app-foundation-operations.md`
- Test: `backend/tests/acceptance.rs` (ignored by default)

**Interfaces:**
- Produces: `docker compose up --build` reaching a healthy application on `http://localhost:8000`.

- [ ] **Step 1: Write the failing acceptance test**

```rust
// backend/tests/acceptance.rs — run with: cargo test --test acceptance -- --ignored --test-threads=1
use std::process::Command;

fn compose(args: &[&str]) -> std::process::Output {
    Command::new("docker").arg("compose").args(args)
        .current_dir("/workspace").output().expect("docker compose")
}

#[test]
#[ignore = "drives the full Compose stack; run explicitly"]
fn clean_start_then_restart_against_the_existing_volume() {
    // Spec section 15, the first ADR-001 evidence row.
    compose(&["down", "-v"]);
    let up = compose(&["up", "--build", "-d", "--wait"]);
    assert!(up.status.success(), "{}", String::from_utf8_lossy(&up.stderr));

    let body = ureq::get("http://localhost:8000/health/ready").call().unwrap();
    assert_eq!(body.status(), 200);

    // Write a row through the database, then restart everything.
    let seed = compose(&["exec", "-T", "db", "psql", "-U", "postgres", "-d", "fau",
        "-c", "insert into tenants (id, name, status) values (gen_random_uuid(),'persist','active')"]);
    assert!(seed.status.success());

    compose(&["down"]); // keeps the volume
    let again = compose(&["up", "-d", "--wait"]);
    assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));

    let count = compose(&["exec", "-T", "db", "psql", "-U", "postgres", "-d", "fau", "-tAc",
        "select count(*) from tenants where name = 'persist'"]);
    assert_eq!(String::from_utf8_lossy(&count.stdout).trim(), "1", "content was not preserved");

    assert_eq!(ureq::get("http://localhost:8000/health/ready").call().unwrap().status(), 200);
}

#[test]
#[ignore = "drives the full Compose stack; run explicitly"]
fn the_migrate_service_completes_before_the_app_starts() {
    compose(&["down", "-v"]);
    compose(&["up", "--build", "-d", "--wait"]);
    let ps = compose(&["ps", "-a", "--format", "{{.Service}} {{.State}} {{.ExitCode}}"]);
    let text = String::from_utf8_lossy(&ps.stdout);
    assert!(text.contains("migrate exited 0"), "migrate did not complete cleanly: {text}");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd backend && cargo test --test acceptance -- --ignored`
Expected: FAIL — no `compose.yaml`.

- [ ] **Step 3: Write `compose.yaml`**

```yaml
# The FAU application stack. The agent-box stack is /workspace/docker-compose.yml
# and always needs -f; Compose prefers this file, so a bare `docker compose down`
# cannot take down the container the work is happening in. That is deliberate.
name: fau-app

services:
  db:
    image: postgres:17.5-bookworm@sha256:2088c1744625793a8a89118d2dee63fb121139141ff0ff53bd72e63bb6089d0d
    environment:
      POSTGRES_DB: fau
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-postgres}
    ports:
      - "127.0.0.1:5433:5432"
    volumes:
      - db-data:/var/lib/postgresql/data
      - ./backend/db:/docker-entrypoint-initdb.d:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres -d fau"]
      interval: 2s
      timeout: 2s
      retries: 30

  migrate:
    build:
      context: .
      dockerfile: Dockerfile
      args:
        FAU_BUILD_REVISION: ${FAU_BUILD_REVISION:-dev}
    image: fau/app:dev
    command: ["migrate"]
    environment:
      MIGRATION_DATABASE_URL: postgres://fau_migrate:${FAU_MIGRATE_PASSWORD:-fau_migrate}@db:5432/fau
      LOG_LEVEL: ${LOG_LEVEL:-info}
    depends_on:
      db:
        condition: service_healthy
    restart: "no"

  app:
    image: fau/app:dev
    command: ["serve"]
    environment:
      APP_ENV: development
      HTTP_BIND: 0.0.0.0:8000
      PUBLIC_BASE_URL: http://localhost:8000
      DATABASE_URL: postgres://fau_app:${FAU_APP_PASSWORD:-fau_app}@db:5432/fau
      DB_POOL_MAX_CONNECTIONS: ${DB_POOL_MAX_CONNECTIONS:-10}
      LOG_LEVEL: ${LOG_LEVEL:-info}
      MAIL_TRANSPORT: smtp://mail:1025
      SIGNUP_NOTIFICATION_TO: fau-local@example.test
    ports:
      - "127.0.0.1:8000:8000"
    depends_on:
      db:
        condition: service_healthy
      migrate:
        condition: service_completed_successfully
    read_only: true
    tmpfs:
      - /tmp
    healthcheck:
      # No curl in the runtime stage, so the binary is not the probe's dependency —
      # Compose only needs to know the port answers.
      test: ["CMD-SHELL", "exec 3<>/dev/tcp/127.0.0.1/8000 && printf 'GET /health/ready HTTP/1.0\\r\\n\\r\\n' >&3 && head -1 <&3 | grep -q '200'"]
      interval: 3s
      timeout: 2s
      retries: 20

  mail:
    # Local test adapter. No external delivery, by design (spec section 1).
    image: axllent/mailpit:v1.20
    ports:
      - "127.0.0.1:8025:8025"
    environment:
      MP_SMTP_AUTH_ACCEPT_ANY: "true"
      MP_SMTP_AUTH_ALLOW_INSECURE: "true"

volumes:
  db-data:
```

`backend/db/compose-init.sh` runs after `roles.sql` (initdb scripts run in filename order, so name it
`zz-compose-init.sh`) and grants login plus the local passwords:

```sh
#!/bin/sh
# Local development only. Production credentials are provisioned by #3424.
set -eu
psql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" <<SQL
alter role fau_migrate login password '${FAU_MIGRATE_PASSWORD:-fau_migrate}';
alter role fau_app     login password '${FAU_APP_PASSWORD:-fau_app}';
SQL
```

- [ ] **Step 4: Append the application section to `.env.example`**

```sh
# ---------------------------------------------------------------------------
# FAU application stack (compose.yaml). The variables above belong to the
# agent-box stack (docker-compose.yml); one .env serves both, and each Compose
# file ignores the other's variables.
#
# Local development values only. Nothing here is a production credential.
# ---------------------------------------------------------------------------
POSTGRES_PASSWORD=postgres
FAU_MIGRATE_PASSWORD=fau_migrate
FAU_APP_PASSWORD=fau_app
DB_POOL_MAX_CONNECTIONS=10
LOG_LEVEL=info
```

- [ ] **Step 5: Write `docs/app-foundation-operations.md`**

Cover, each in a short section: the acceptance command; the upgrade path for an existing volume
(**a previously completed one-shot `migrate` container does not re-run on its own** — Compose
restarts it only because `depends_on … service_completed_successfully` re-evaluates on `up`, so the
documented command is `docker compose up -d --wait`, and `docker compose run --rm migrate` is the
manual fallback); role provisioning as the production precondition for #3424; the two lock
variables and when to raise them; why `compose.yaml` and `docker-compose.yml` are different stacks;
and how to run the ignored image and acceptance tests.

- [ ] **Step 6: Run the acceptance tests**

Run: `cd backend && cargo test --test acceptance -- --ignored --test-threads=1`
Expected: PASS.

- [ ] **Step 7: Full suite**

Run: `cd backend && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Then the ignored ones: `cargo test -- --ignored --test-threads=1`
Expected: all green.

- [ ] **Step 8: Leave in the working tree**

---

## Self-Review

**Spec coverage.** Every section of `docs/app-foundation-design.md` maps to a task:
§2 layout → Task 1; §3 process contract → Tasks 1, 4, 8; §4 configuration → Task 2;
§5 migrations and contract → Tasks 4, 6, 7; §6 database roles → Task 5; §7 RLS → deliberately
absent, asserted only negatively in Task 5's `rolbypassrls` test; §8 HTTP surface → Task 8;
§9 health and readiness → Tasks 8, 9; §10 logging → Task 10; §11 error contract → Task 8;
§12 shutdown → Task 11; §13 image → Task 12; §14 Compose → Task 13; §15 test plan → every task
(the mapping is in the table below); §16 open items → the decisions table at the top.

Spec §15 row-by-row:

| Spec test row | Task |
| --- | --- |
| Clean checkout, then restart against an existing volume | 13 |
| Image contract | 12 |
| Database down, then back | 9 |
| Sentinel secrets never in log output | 10 |
| Migration checksum altered after being applied | 4 |
| Two `migrate` processes concurrently | 4 |
| Migration lock held beyond the bound | 4 |
| Cross-tenant reference inserted directly in SQL | 6 |
| Role assignment with `ends_on_exclusive` ≤ `starts_on` | 6 |
| Contract below the binary's minimum | 7 |
| Contract above the binary's minimum | 7 |
| Key material in an application table | 6 |

**Placeholder scan.** No TBD, no "add error handling", no "similar to Task N". The two ignored test
files are marked ignored deliberately, not left unwritten.

**Type consistency.** `ReadinessState` is introduced in Task 8's `AppState` and implemented in Task 9
— Task 8 stubs it as always-ready and Task 9 replaces the body, which is noted in Task 8's
Interfaces block. `MIGRATION_LOCK_ID` is `pub` in `persistence` (Task 4) because Task 4's own test
contends on it. `TestDb` grows `app_pool` in Task 5 and `sever_connections`/`restore_connections` in
Task 9; both are named in the task that adds them.

---

## Handover

The git guardrail in `~/.claude/CLAUDE.md` is still live, so no task commits. All work stays in the
working tree, and the final report lists the paths to commit.
