# FAU key service: operations

Cluster manifests, Helm values and the deletion job live in `ops/openbao/`. Nothing in that
directory is applied yet — deploying it is #3424's, gated on Erik's go-ahead. This runbook covers
first start, day-to-day operation, and the alerts.

## 1. What it is

OpenBao's transit secrets engine, run as FAU's key service: envelope encryption for every
shreddable unit (an FAU's records, a document, a chat month, messages). Full design, threat model
and rationale are in `docs/key-service-design.md`; this document is the operational complement —
how to run it, not why it is shaped this way.

## 2. First start

**`bao operator init`, `unseal` and `generate-root` against the real (non-dev) OpenBao are Erik's
alone, on his own terminal, never through an agent.** Their output is the crown jewels, and an
agent transcript persists (docs/key-service-design.md §4.2).

```bash
kubectl -n openbao exec -it openbao-0 -- bao operator init -key-shares=1 -key-threshold=1
```

This runs `bao` inside the pod, which already has `BAO_ADDR=https://127.0.0.1:8200` and
`BAO_CACERT=/openbao/userconfig/openbao-server-tls/ca.crt` set as container environment (the
chart's default `BAO_ADDR` plus `server.extraEnvironmentVars` in `ops/openbao/helm-values.yaml`,
Task 9 fix round 1) — no extra export needed for this command. The server certificate carries a
`127.0.0.1`/`localhost` SAN (`ops/openbao/k8s/certificates.yaml`) precisely so this in-pod TLS
connection validates.

- Store the unseal key in Proton Pass as "OpenBao unseal key" (#3481).
- Use the initial root token for step 3 below, then revoke it. No standing all-powerful token
  should exist afterwards.

## 3. Configure

Run after the first start, and again after any change under `ops/openbao/` (policies, auth
roles, quotas):

```bash
kubectl -n openbao port-forward svc/openbao 8200:8200 &
# Copy out only ca.crt — never tls.key — so no private key is printed to the terminal.
kubectl -n openbao get secret openbao-server-tls -o jsonpath='{.data.ca\.crt}' \
  | base64 -d > /tmp/openbao-ca.crt
export BAO_ADDR=https://127.0.0.1:8200 BAO_CACERT=/tmp/openbao-ca.crt
export BAO_TOKEN=<root token>          # typed, never pasted into an agent session
sh ops/openbao/configure.sh cluster
bao token revoke -self
rm -f /tmp/openbao-ca.crt
```

The port-forwarded connection is to `127.0.0.1` from outside the pod, over the same listener, so
it needs the same `127.0.0.1` SAN the certificate now carries (Task 9 fix round 1) and an
explicit `BAO_CACERT`, since nothing on the operator's own machine trusts OpenBao's internal CA
by default.

`configure.sh cluster` is idempotent: enables the transit engine and the `fau-keys-queue/` KV
mount, writes the `fau-app` and `fau-keys-operator` policies, sets the transit rate-limit quota,
and configures Kubernetes auth with the `fau-app` and `fau-keys-operator` roles.

## 4. Unseal after every restart

```bash
kubectl -n openbao exec -it openbao-0 -- bao operator unseal
```

Paste the unseal key from Proton Pass when prompted. Like `init` (§2), this runs inside the pod,
so `BAO_ADDR`/`BAO_CACERT` are already set in the container's environment and the command needs no
extra exports.

## 5. Generating a root token when one is needed

Erik's alone, on his own terminal (§2). Like `init` and `unseal`, it runs inside the pod, so
`BAO_ADDR`/`BAO_CACERT` are already set and no port-forward is needed:

```bash
kubectl -n openbao exec -it openbao-0 -- bao operator generate-root -init
# then, with the unseal key and the OTP from the -init step:
kubectl -n openbao exec -it openbao-0 -- bao operator generate-root
kubectl -n openbao exec -it openbao-0 -- bao operator generate-root -decode=<encoded token> -otp=<otp>
```

Use the decoded token through the §3 port-forward setup (`BAO_ADDR`, `BAO_CACERT`), and revoke it
(`bao token revoke -self`) as soon as the task that needed it is done.

## 6. What sealed means

- Login, authorization and navigation keep working — those never touch OpenBao.
- Every encrypted field answers `content_unavailable` (HTTP 503). The Bokmål source string
  (#3439) is: "Innholdet er midlertidig utilgjengelig. Vi jobber med saken."
- Access requests cannot attach a message until OpenBao is unsealed, because encrypting the
  message needs OpenBao.
- `KeyServiceSealed` pages after 2 minutes sealed (`max(vault_core_unsealed) == 0` for 2m).

## 7. Deleting an FAU, purging a document, restoring

FAU deletion, document purge and restore are one-off Jobs, created from the `fau-keys-finalize`
CronJob template with an edited command:

```bash
kubectl -n openbao create job --from=cronjob/fau-keys-finalize fau-keys-delete-<tenant> \
  --dry-run=client -o yaml > /tmp/job.yaml
# edit /tmp/job.yaml: containers[0].command to
#   ["/bin/sh", "/ops/job.sh", "fau", "<tenant>"]
#   ["/bin/sh", "/ops/job.sh", "document", "<tenant>", "<document>"]
#   ["/bin/sh", "/ops/job.sh", "restore", "<key>"]
kubectl apply -f /tmp/job.yaml
```

- **Deleting an FAU** runs only after ADR-003 7a's confirmation (the recovery contact's
  confirmation and its 7-day wait, or the single-member self-delete case —
  `docs/identity-and-encryption.md` §8). The job soft-deletes every `fau-<tenant>-*` key at once.
- **Hard deletion follows by itself**, 7 days later, through the daily `fau-keys-finalize`
  CronJob (`finalize`), which also destroys due chat-expiry entries queued by the monthly
  `fau-keys-chat-expire` CronJob.
- **`finalize` exits 3** when it refuses to destroy a queued chat-month key whose month ended
  less than 12 months ago and whose queue `reason` is not `fau` — i.e. a chat month reached
  through monthly expiry, not through a governed FAU deletion. `job.sh` execs into `shred.sh`, so
  that exit status becomes the container's exit status; with `restartPolicy: Never` and
  `backoffLimit: 0` this fails the Kubernetes Job, which is what makes the refusal visible as a
  failed Job alongside `KeyServiceShredCritical` — a refusal must never pass silently. Every
  queue entry's `reason` (`fau`, `document`, or `chat-expire`) records which flow queued it; see
  `ops/openbao/shred.sh` for the full state machine.
- **Restore** is available any time before hard deletion (`restore <key>`) and un-queues the key.

`shred.sh`'s exit codes, which `job.sh` passes through as the Job's result; every non-zero one
except 2 logs a `shred: CRITICAL` line:

| Exit | Meaning | What to do |
|---|---|---|
| 0 | Done. Keys already soft-deleted and queued, or gone, are skipped. | Nothing. |
| 1 | OpenBao error: unreachable, sealed, token rejected, permission denied, or a failed read or write. Nothing is treated as "empty" or "gone" unless OpenBao said so, and `finalize` unqueues nothing it could not read. | Read the CRITICAL line, fix the cause (unseal, NetworkPolicy, auth role), re-run the job. Re-running is safe. |
| 2 | Bad input (usage). | Fix the command. |
| 3 | `finalize` refused a young chat month (above). | Investigate the queue entry. |
| 4 | `fau <tenant>` matched no keys: a wrong tenant id, or an FAU that never stored content. | Check the tenant id. If it is right, there was nothing to shred. |

A `shred: CRITICAL re-queued …` line with exit 0 means a key was found soft-deleted but not
queued — an earlier run stopped between its two writes — and has now been queued from this run's
time; its 7-day window starts now.

## 8. Until #3507: volume loss is total

No Raft snapshots are configured — deliberately: a snapshot history of key material would keep
destroyed keys alive after a shred, defeating crypto-shredding. Until #3507 lands the replica,
losing the OpenBao PVC loses every key and so all content, with no recovery path. Acceptable only
while no real FAU data exists yet.

## 9. Alerts

| Alert | Source | Severity | What to do |
|---|---|---|---|
| `KeyServiceSealed` | `max(vault_core_unsealed) == 0` for 2m | critical | OpenBao is sealed. Unseal it (§4). While sealed, all content reads 503; this is expected, not a data-loss signal. |
| `KeyServiceRateLimited` | `increase(vault_quota_rate_limit_violation[15m]) > 0` | warning | The `fau-app` role hit the transit rate-limit quota. Check for a runaway client or a real traffic increase before raising the quota (§4.3, tuned under #3442). |
| `KeyServiceManyDistinctKeysDecrypted` (Loki rule, not yet enabled — #3442 verifies field names against a real audit line first) | audit log, `transit/decrypt/fau-*` count by key over 1h | critical | The mass-decryption signal. Investigate which credential is decrypting many distinct units; this is the pattern a compromised app credential would produce. |
| `KeyServiceSoftDeleteOrRestore` (Loki rule, same caveat) | audit log, any `soft-delete` / `soft-delete-restore` | critical | Every soft-delete or restore is a deliberate, rare action (ADR-003 decision 7). Confirm it corresponds to an authorized deletion, expiry, or restore request. |
| `KeyServiceShredCritical` (Loki rule, same caveat) | audit log, `shred: CRITICAL` lines from the `shred` container | critical | Covers any OpenBao error in a job (exit 1), `finalize`'s refusal to destroy a young chat month (exit 3), `fau` matching no keys (exit 4), a malformed queue entry, a key found queued but not soft-deleted, and a soft-deleted key re-queued (all in §7). Read the log line for which case, and check `ops/openbao/shred.sh`. |

The Loki rules are commented out in `ops/openbao/k8s/alerts.yaml` pending that field-name
verification; only the two Prometheus rules are live in the `PrometheusRule` object as written.

## 10. Local development

```bash
docker compose -f compose.yaml up -d openbao
docker compose -f compose.yaml run --rm openbao-config
```

- Dev tokens are fixed: `dev-only-root` (root), `dev-only-app` (the `fau-app` policy),
  `dev-only-operator` (the `fau-keys-operator` policy). Never use these outside compose.
- The dev server is in-memory (`-dev`) and keeps nothing across restarts; `openbao-config` reruns
  `configure.sh dev` idempotently against it.
- `ops/openbao/test-shred.sh` exercises `shred.sh` against the dev server; run it with
  `docker compose -f compose.yaml exec -T -e BAO_TOKEN=dev-only-root openbao sh /ops/test-shred.sh`.

## 11. After every server certificate renewal: reload with SIGHUP

cert-manager renews `openbao-server` (`ops/openbao/k8s/certificates.yaml`, 90 days, renewed 15
days before expiry, so about every 75 days) and updates the `openbao-server-tls` Secret, but
OpenBao reads its listener certificate only at start and on SIGHUP. Until it is signalled it keeps
serving the old certificate, and clients start failing when that one expires.

**A restart is not the remedy: a restarted OpenBao comes up sealed**, and every content request
answers `content_unavailable` until Erik unseals it (§4, §6). Send SIGHUP instead, which reloads
the listener certificates (and the declared audit device) without sealing.

1. See when renewal happened or is due:

   ```bash
   kubectl -n openbao get certificate openbao-server \
     -o jsonpath='{.status.renewalTime}{"  notAfter="}{.status.notAfter}{"\n"}'
   ```

2. Wait until the pod sees the renewed file — the kubelet refreshes a mounted Secret within a
   minute or two. Compare the two hashes (certificates only, never `tls.key`):

   ```bash
   kubectl -n openbao get secret openbao-server-tls -o jsonpath='{.data.tls\.crt}' | base64 -d | sha256sum
   kubectl -n openbao exec openbao-0 -- sha256sum /openbao/userconfig/openbao-server-tls/tls.crt
   ```

3. Signal the server. PID 1 in the chart's container is the `/bin/sh -ec` wrapper that starts
   `bao`, not `bao` itself — a SIGHUP to PID 1 would kill the shell and restart the pod, sealing
   it — so signal the `bao` process by name, as the chart's own `preStop` hook does:

   ```bash
   kubectl -n openbao exec openbao-0 -- sh -c 'kill -HUP "$(pidof bao)"'
   ```

4. Confirm the new certificate is served, through the §3 port-forward:

   ```bash
   openssl s_client -connect 127.0.0.1:8200 -servername openbao.openbao.svc </dev/null 2>/dev/null \
     | openssl x509 -noout -serial -enddate
   kubectl -n openbao get secret openbao-server-tls -o jsonpath='{.data.tls\.crt}' | base64 -d \
     | openssl x509 -noout -serial -enddate
   ```

   The serials and end dates must match, and `bao status` must still report `Sealed false`.

An alert on the served certificate nearing expiry — which is what catches a missed SIGHUP —
belongs to #3442.
