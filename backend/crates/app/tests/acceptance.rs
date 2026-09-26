//! Acceptance test for the local Compose application stack (design section 14; spec
//! section 15's first ADR-001 evidence row -- "clean checkout, then restart against
//! an existing database volume" -- plus a check that `migrate` completes cleanly
//! rather than being left running or failed).
//!
//! Drives the real `docker compose` CLI against the real, root `compose.yaml`. Every
//! invocation here runs under its own Compose project, `fau-acceptance`, with its own
//! fixed, non-default host ports (`FAU_DB_PORT=15433`, `FAU_APP_PORT=18000`,
//! `FAU_MAIL_PORT=18025`) -- **never** the developer's own `fau-app` project, whose
//! `db` container (`fau-app-db-1`) this agent container's network attachment must
//! survive this test run. A stray `docker compose down` against `fau-app`, or against
//! its `fau-app_default` network, is not just a test bug: it can sever this very
//! session. See the module-level `compose`/`compose_ok` helpers below, which always
//! pass an explicit `-p fau-acceptance -f <repo>/compose.yaml`.
//!
//! This agent container cannot reach the host's `127.0.0.1`, where Compose publishes
//! `FAU_APP_PORT` -- so this test never makes an HTTP request of its own. Readiness
//! evidence instead comes from the `app` container's own Docker healthcheck (which
//! itself runs a `GET /health/ready` *inside* the container), read back with `docker
//! inspect .State.Health.Status`. No `ureq` dependency as a result.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

/// A dedicated project, distinct from the developer's own `fau-app` stack, so this
/// test can never collide with -- or tear down -- the dev database this agent
/// container is attached to.
const PROJECT: &str = "fau-acceptance";

/// Fixed and distinct from the dev stack's own defaults (5433/8000/8025), so a stray
/// port collision can never make this test silently observe the *wrong* stack.
const PORT_ENV: &[(&str, &str)] = &[
    ("FAU_DB_PORT", "15433"),
    ("FAU_APP_PORT", "18000"),
    ("FAU_MAIL_PORT", "18025"),
];

/// The repository root, derived from this crate's own manifest location
/// (`backend/crates/app`) rather than a hard-coded `/workspace`.
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent() // backend/crates
        .expect("crates/ parent")
        .parent() // backend
        .expect("backend/ parent")
        .parent() // repository root
        .expect("repository root")
        .to_path_buf()
}

fn compose_file() -> PathBuf {
    repo_root().join("compose.yaml")
}

/// Runs `docker compose -p fau-acceptance -f <repo>/compose.yaml <args>`, with
/// [`PORT_ENV`] layered on top of this process's own environment. Always pins both
/// the project name and the compose file explicitly -- never relies on the current
/// directory or on Compose's own file-discovery order, which is exactly what would
/// let this accidentally target `fau-app`.
fn compose(args: &[&str]) -> Output {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-p")
        .arg(PROJECT)
        .arg("-f")
        .arg(compose_file());
    cmd.args(args);
    for (key, value) in PORT_ENV {
        cmd.env(key, value);
    }
    cmd.output().expect("run docker compose")
}

fn compose_ok(args: &[&str]) -> Output {
    let out = compose(args);
    assert!(
        out.status.success(),
        "docker compose {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    out
}

/// Tears down only the `fau-acceptance` project on drop -- `down -v`, so the volume
/// never survives to leak into the next run -- whether the test that created it
/// passed, failed an assertion, or panicked. Never touches `fau-app`.
struct AcceptanceStackGuard;

impl Drop for AcceptanceStackGuard {
    fn drop(&mut self) {
        let _ = compose(&["down", "-v"]);
    }
}

fn container_id(service: &str) -> String {
    let out = compose_ok(&["ps", "-q", service]);
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// `docker inspect --format '{{.State.Health.Status}}' <container>` -- the
/// readiness evidence Compose itself acts on. Returns an empty string if the container does not exist yet
/// or carries no healthcheck.
fn health_status(container: &str) -> String {
    let out = Command::new("docker")
        .args(["inspect", "--format", "{{.State.Health.Status}}", container])
        .output()
        .expect("docker inspect");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Polls the `app` container's own healthcheck until it reports `healthy`, or panics
/// with the service's logs after `timeout`. This is the proof that
/// `GET /health/ready` answered 200 *inside* the container -- see the module doc for
/// why this test cannot make that request itself.
fn wait_for_app_healthy(timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let id = container_id("app");
        if !id.is_empty() {
            let status = health_status(&id);
            if status == "healthy" {
                return;
            }
            if status == "unhealthy" {
                let logs = compose(&["logs", "app"]);
                panic!(
                    "app container is unhealthy; logs:\n{}",
                    String::from_utf8_lossy(&logs.stdout)
                );
            }
        }
        if Instant::now() >= deadline {
            let ps = compose(&["ps", "-a"]);
            let logs = compose(&["logs", "app"]);
            panic!(
                "app container did not become healthy within {timeout:?}\nps:\n{}\nlogs:\n{}",
                String::from_utf8_lossy(&ps.stdout),
                String::from_utf8_lossy(&logs.stdout)
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

#[test]
#[ignore = "drives the full Compose stack; run explicitly with --test-threads=1"]
fn clean_start_then_restart_against_the_existing_volume() {
    // Stale leftovers from an earlier aborted run, before the guard below exists to
    // cover the rest of this test.
    let _ = compose(&["down", "-v"]);
    let _guard = AcceptanceStackGuard;

    let up = compose(&["up", "--build", "-d", "--wait"]);
    assert!(
        up.status.success(),
        "docker compose up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );
    wait_for_app_healthy(Duration::from_secs(90));

    // Write a row through the database directly, then restart everything and prove
    // it survived -- spec section 15's "content preserved" evidence row. The tenants
    // table's NOT NULL columns per migration 0002 (id, name, status) plus 0004's
    // school_id foreign key (#3441), so a municipality and a school are seeded first.
    let seed = compose_ok(&[
        "exec",
        "-T",
        "db",
        "psql",
        "-U",
        "postgres",
        "-d",
        "fau",
        "-c",
        "insert into municipalities (id, name, county_number, county_name, slug, status, source, search_text) \
         values ('01990000-0000-7000-8000-000000000301', 'Oslo', '03', 'Oslo', '0301-oslo', 'active', 'manual', 'oslo'); \
         insert into municipality_numbers (municipality_id, number, valid_from) \
         values ('01990000-0000-7000-8000-000000000301', '0301', '1838-01-01'); \
         insert into schools (id, municipality_id, origin, display_name, verification, orgnr, status, search_text) \
         values ('01990000-0000-7000-8000-000000000302', '01990000-0000-7000-8000-000000000301', 'register', \
                 'Persist skole', 'listed', '999999999', 'active', 'persist skole'); \
         insert into tenants (id, name, status, school_id) \
         values (gen_random_uuid(), 'persist', 'active', '01990000-0000-7000-8000-000000000302')",
    ]);
    assert!(seed.status.success());

    compose_ok(&["down"]); // no -v: the volume must survive this restart
    let again = compose(&["up", "-d", "--wait"]);
    assert!(
        again.status.success(),
        "docker compose up (restart) failed: {}",
        String::from_utf8_lossy(&again.stderr)
    );
    wait_for_app_healthy(Duration::from_secs(90));

    let count = compose_ok(&[
        "exec",
        "-T",
        "db",
        "psql",
        "-U",
        "postgres",
        "-d",
        "fau",
        "-tAc",
        "select count(*) from tenants where name = 'persist'",
    ]);
    assert_eq!(
        String::from_utf8_lossy(&count.stdout).trim(),
        "1",
        "content was not preserved across the restart"
    );
}

/// This does not independently observe *ordering* (that guarantee lives in
/// `compose.yaml`'s `depends_on: migrate: condition: service_completed_successfully`,
/// declarative config this test does not re-derive from timestamps) -- it proves the
/// weaker, directly checkable half: that once the stack is up and `app` is healthy,
/// `migrate` is among the containers and exited 0, never left running or failed.
#[test]
#[ignore = "drives the full Compose stack; run explicitly with --test-threads=1"]
fn the_migrate_service_exits_zero() {
    let _ = compose(&["down", "-v"]);
    let _guard = AcceptanceStackGuard;

    let up = compose(&["up", "--build", "-d", "--wait"]);
    assert!(
        up.status.success(),
        "docker compose up failed: {}",
        String::from_utf8_lossy(&up.stderr)
    );
    wait_for_app_healthy(Duration::from_secs(90));

    let ps = compose_ok(&[
        "ps",
        "-a",
        "--format",
        "{{.Service}} {{.State}} {{.ExitCode}}",
    ]);
    let text = String::from_utf8_lossy(&ps.stdout);
    assert!(
        text.contains("migrate exited 0"),
        "migrate did not complete cleanly: {text}"
    );
}
