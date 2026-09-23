//! The production container image contract (design section 13). Builds the real
//! image with the real Dockerfile and inspects the result — no bind mounts, since
//! the docker CLI here talks to the *host* daemon and a bind mount would resolve on
//! the host, not in this container. `image_contract` is slow (a full release build)
//! and mutates the host's local image cache, so it is `#[ignore]`d: run explicitly
//! with `cargo test -p fau-app --test image -- --ignored`.
//! `dockerfile_pins_base_images_by_digest` needs neither the docker daemon nor a
//! build, so it runs in the ordinary suite.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Duration;

const IMAGE: &str = "fau/app:dev";

fn docker(args: &[&str]) -> Output {
    Command::new("docker")
        .args(args)
        .output()
        .expect("run docker")
}

fn docker_ok(args: &[&str]) -> Output {
    let out = docker(args);
    assert!(
        out.status.success(),
        "docker {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    out
}

/// The repository root, derived from this crate's own manifest location
/// (`backend/crates/app`) rather than a hard-coded `/workspace` — this test must
/// also make sense if the repository is ever checked out somewhere else.
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

/// Removes the container on drop, whether the test that created it passed, failed
/// an assertion, or panicked. `-f` copes with a container that is still running.
struct ContainerGuard(String);

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        let _ = Command::new("docker").args(["rm", "-f", &self.0]).output();
    }
}

/// A plain string test on the Dockerfile text, not a build: every `FROM` line must
/// pin its base image by digest, so a moving tag can never silently change what
/// ships across test, migration and deploy.
#[test]
fn dockerfile_pins_base_images_by_digest() {
    let dockerfile_path = repo_root().join("Dockerfile");
    let dockerfile = fs::read_to_string(&dockerfile_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", dockerfile_path.display()));
    let from_lines: Vec<&str> = dockerfile
        .lines()
        .map(str::trim)
        .filter(|line| line.to_uppercase().starts_with("FROM "))
        .collect();
    assert!(!from_lines.is_empty(), "no FROM lines found in Dockerfile");
    for line in from_lines {
        assert!(
            line.contains("@sha256:"),
            "base image not pinned by digest: {line}"
        );
    }
}

#[test]
#[ignore = "builds a container image; run explicitly"]
fn image_contract() {
    let root = repo_root();
    let build_revision = "image-contract";
    let build = Command::new("docker")
        .args([
            "build",
            "-t",
            IMAGE,
            "-f",
            "Dockerfile",
            "--build-arg",
            &format!("FAU_BUILD_REVISION={build_revision}"),
            ".",
        ])
        .current_dir(&root)
        .status()
        .expect("docker build");
    assert!(build.success(), "docker build failed");

    // --version reads no configuration and works even under the runtime contract:
    // read-only root filesystem, tmpfs /tmp, non-root uid. It also carries the
    // build revision passed above, proving the image actually embeds it rather
    // than a stale or default binary.
    let out = docker(&[
        "run",
        "--rm",
        "--read-only",
        "--tmpfs",
        "/tmp",
        "--user",
        "10001:10001",
        IMAGE,
        "--version",
    ]);
    assert!(out.status.success(), "fau --version failed: {out:?}");
    let version_out = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        version_out.starts_with("fau "),
        "unexpected --version output: {version_out:?}"
    );
    assert!(
        version_out.contains(build_revision),
        "--version output does not carry the build revision: {version_out:?}"
    );

    // Non-root by default, not only when asked.
    let uid = docker_ok(&["run", "--rm", "--entrypoint", "id", IMAGE, "-u"]);
    let uid = String::from_utf8_lossy(&uid.stdout).trim().to_string();
    assert_ne!(uid, "0", "image runs as root");
    assert_eq!(uid, "10001", "unexpected uid");

    // The read-only-root-filesystem contract, tested for real: `serve` under
    // `--read-only --tmpfs /tmp` with a database it can never reach (readiness
    // handles that; it is not a startup failure) must stay running, and must exit 0
    // when asked to stop. Checking that uid 10001 cannot `touch /nope` proves
    // nothing about `--read-only` on its own -- that write would fail for that uid
    // regardless of the flag -- so this exercises the actual runtime contract
    // instead: the process itself, writing wherever it normally would (stdout,
    // logs, tmp files), under the exact flags the container will run with.
    let container_name = format!("fau-image-contract-{}", std::process::id());
    let _guard = ContainerGuard(container_name.clone());
    docker_ok(&[
        "run",
        "-d",
        "--name",
        &container_name,
        "--read-only",
        "--tmpfs",
        "/tmp",
        "-e",
        "APP_ENV=development",
        "-e",
        "PUBLIC_BASE_URL=http://localhost",
        "-e",
        "DATABASE_URL=postgres://x:y@127.0.0.1:1/z",
        "-e",
        "DB_POOL_MAX_CONNECTIONS=1",
        IMAGE,
    ]);

    std::thread::sleep(Duration::from_secs(3));

    let running = docker_ok(&["inspect", "--format", "{{.State.Running}}", &container_name]);
    assert_eq!(
        String::from_utf8_lossy(&running.stdout).trim(),
        "true",
        "serve did not stay running under --read-only --tmpfs /tmp with an unreachable database"
    );

    // 30s: comfortably above the process's own 25s bounded drain (design section
    // 12), so a slow-but-clean shutdown is never mistaken for a hung one.
    docker_ok(&["stop", "-t", "30", &container_name]);
    let exit_code = docker_ok(&[
        "inspect",
        "--format",
        "{{.State.ExitCode}}",
        &container_name,
    ]);
    assert_eq!(
        String::from_utf8_lossy(&exit_code.stdout).trim(),
        "0",
        "serve did not exit 0 after SIGTERM"
    );
    drop(_guard);

    // The runtime stage carries the binary, CA certificates and migrations — no
    // build toolchain, and no /src at all (the build stage's workspace copy).
    for absent in [
        "/usr/local/cargo",
        "/usr/bin/gcc",
        "/usr/local/rustup",
        "/src",
    ] {
        let probe = docker(&[
            "run",
            "--rm",
            "--entrypoint",
            "sh",
            IMAGE,
            "-c",
            &format!("test ! -e {absent}"),
        ]);
        assert!(
            probe.status.success(),
            "{absent} is present in the runtime stage"
        );
    }

    // CA certificates are present, so the binary can make outbound TLS connections
    // (the database connection uses tls-rustls, which reads this store).
    let ca = docker(&[
        "run",
        "--rm",
        "--entrypoint",
        "sh",
        IMAGE,
        "-c",
        "test -s /etc/ssl/certs/ca-certificates.crt",
    ]);
    assert!(
        ca.status.success(),
        "CA certificate bundle missing from the image"
    );

    let migrations = docker(&[
        "run",
        "--rm",
        "--entrypoint",
        "sh",
        IMAGE,
        "-c",
        "ls /app/migrations/0002_identity_and_tenancy.sql",
    ]);
    assert!(
        migrations.status.success(),
        "migrations/ missing from the image"
    );

    // No secret, build artifact or VCS metadata reached the image, searched rather
    // than probed one hard-coded path at a time. `-xdev` keeps this to the image's
    // own filesystem (it will not descend into the tmpfs mounted over /tmp, nor
    // into /proc or /sys, which are separate filesystems of their own). This is an
    // audit of what reached the image, not of what the app's own uid can see at
    // runtime, so it runs as `--user 0`: uid 10001 cannot read `/root`
    // (`drwx------`), and anything planted there would silently pass a search run
    // as that uid. Run as root, `find` needs no `2>/dev/null` to hide
    // permission-denied noise -- there is none left to hide -- but the redirect is
    // kept anyway so the check still degrades safely (fails on the stdout
    // assertion, not on an unrelated stderr surprise) if a future image adds a
    // filesystem root cannot fully read either.
    let secret_search = docker(&[
        "run",
        "--rm",
        "--user",
        "0",
        "--entrypoint",
        "sh",
        IMAGE,
        "-c",
        "find / -xdev \\( -name '.env*' -o -name .git -o -name target \\) -print 2>/dev/null",
    ]);
    let matches = String::from_utf8_lossy(&secret_search.stdout);
    assert!(
        matches.trim().is_empty(),
        "secret, VCS or build-artifact paths found in the image: {matches}"
    );

    // The `test-routes` feature (only used by `tests/panic_handling.rs` etc.) must
    // never be compiled into the shipped binary. `grep -q` alone is not enough: it
    // also exits non-zero (1) if the pattern is absent *or* if grep itself is
    // missing or the file unreadable, which would make `! grep -q ...` pass
    // vacuously in exactly the failure case this is meant to catch. Checking the
    // exit code explicitly distinguishes "not found" (1, wanted) from "found" (0)
    // or "error" (2, e.g. `/app/fau` unreadable). grep is present in
    // `debian:bookworm-slim` because `grep` is its own package with
    // `Priority: required`, so every Debian install carries it -- not because it
    // happens to ride along with some other package, and not merely because it is
    // commonly installed.
    let no_test_routes = docker(&[
        "run",
        "--rm",
        "--entrypoint",
        "sh",
        IMAGE,
        "-c",
        "grep -q '/test/slow' /app/fau; test $? -eq 1",
    ]);
    assert!(
        no_test_routes.status.success(),
        "the image binary contains the test-routes feature's /test/slow route (or grep could not run)"
    );
}
