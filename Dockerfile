# syntax=docker/dockerfile:1.7
# FAU application image (design section 13). Dockerfile.agent is the agent-box image
# and is unrelated -- do not confuse the two build contexts.
#
# Multi-stage: base images are pinned by digest, so a moving tag can never silently
# change what ships across test, migration and deploy. The runtime stage carries
# only the binary, CA certificates and migrations/ -- no compiler, no source, no
# secrets. It runs as a non-root user with a read-only root filesystem and a tmpfs
# /tmp, per the process contract.

FROM rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS build
WORKDIR /src

# Manifests and workspace metadata first, so a source-only change does not bust the
# dependency-resolution cache layer.
COPY backend/Cargo.toml backend/Cargo.lock backend/rust-toolchain.toml ./
COPY backend/crates ./crates
# crates/persistence/build.rs tracks `../../migrations` -- it must be a sibling of
# crates/ inside the build stage, exactly as it is in backend/.
COPY backend/migrations ./migrations

ARG FAU_BUILD_REVISION=unknown
ENV FAU_BUILD_REVISION=${FAU_BUILD_REVISION}

# --locked: the image is built from exactly Cargo.lock, never a re-resolved graph.
# No --features: the `test-routes` feature (fau-app) must never be enabled here.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bin fau \
 && cp /src/target/release/fau /fau

FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251 AS runtime

# No apt-get: `useradd` is already present (shadow-utils/passwd is Priority: required,
# so it ships even in the slim base), and the CA bundle is copied from the build
# stage below instead of installed here -- one less package manager invocation, one
# less layer, and no apt cache to remember to clean up.
RUN useradd --system --uid 10001 --no-create-home --shell /usr/sbin/nologin fau

WORKDIR /app
# The `rust:*-bookworm` build image already carries a populated CA bundle (it is a
# full Debian image, not the slim runtime base); reusing it here is simpler and no
# less trustworthy than installing the same `ca-certificates` package a second time.
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /fau /app/fau
# Copied so the exact applied SQL can be inspected inside a running container. The
# binary embeds the same files at compile time via `sqlx::migrate!`; these files on
# disk are evidence for operators, not the source of truth the binary reads from.
COPY backend/migrations /app/migrations

USER 10001:10001
EXPOSE 8000
ENTRYPOINT ["/app/fau"]
CMD ["serve"]
