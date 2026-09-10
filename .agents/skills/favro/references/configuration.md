# Project configuration

## Discovery and secrets

The CLI resolves configuration in this order:

1. `--config <path>`
2. `FAVRO_PROJECT_CONFIG`
3. `.favro/project.toml` in the current directory
4. Parent directories

Commit `project.toml`. Unknown keys are rejected so typos and removed compatibility settings cannot silently fall back to defaults. Never put tokens or passwords in it. Authentication profiles name environment variables; actual values live in the environment, `FAVRO_ENV_FILE`, or an ignored env file. Env-file loading imports only `FAVRO_*` keys. It uses the first readable file in this order: `FAVRO_ENV_FILE` alone when set; otherwise configured files, `favro.env`, then `.env`.

```toml
[project]
collection = "Example product"
visibility = "users" # users | organization | public
default_human = "owner"

[workflow]
lanes = ["Inbox", "Backlog", "Ready", "In Progress", "Waiting", "Blocked", "Review", "Done"]
inbox = "Inbox"
backlog = "Backlog"
ready = "Ready"
doing = "In Progress"
waiting = "Waiting"
blocked = "Blocked"
review = "Review"
done = "Done"

[interviews]
card_granularity = "topic" # topic | question

[templates]
work = "Purpose/outcome:\nNext action:\nAcceptance:"
code = "Change:\nAcceptance:"

[completion]
require_result = true
require_review = true

[history]
require_archive_reference = true
path = "docs/project-history.md"

# Optional ignored env files; the first readable file wins before local favro.env/.env:
[credentials]
env_files = ["/secure/project/favro.env"]

[auth.primary]
email_env = "FAVRO_PRIMARY_EMAIL"
token_env = "FAVRO_PRIMARY_TOKEN"
organization_id_env = "FAVRO_PRIMARY_ORG_ID"
user_id = "agent Favro user ID"

[participants.owner]
user_id = "human Favro user ID"
email_env = "FAVRO_OWNER_EMAIL"

[roles.product]
emoji = "🧩"
label = "Product"
board = "Product"
auth = "primary"

[fields.priority]
name = "Priority"

[fields.complexity]
name = "Complexity"
```

Multiple roles may share an authentication profile while retaining separate role identities. Prefer separate Favro accounts when available. Participants are assignable people; authentication profiles are principals the CLI can act as. Do not conflate them.

Treat role emojis as immutable project identifiers after work begins. The committed `project.toml` and its history are authoritative. As a local early-warning check, `ensure-project` records each role’s emoji in the machine cache and later local `check`/`ensure-project` calls reject changes; clearing that cache or using another machine removes only this auxiliary check. The CLI always rejects duplicate emojis.

## Shared fields

Use existing native shared fields:

- Priority: Status/Multiple Select with `Critical`, `High`, `Normal`, `Low`; default Normal. Critical requires rationale in the card.
- Complexity: Rating from 1 (low complexity) through 5 (high complexity). It communicates difficulty/uncertainty, never elapsed time.
- Ownership: native card assignments, not a text field.

Favro's published REST API can read custom-field definitions and set values on cards, but does not document creating definitions. `favro check` discovers configured fields by ID/name and validates their types. If missing, create them manually in Favro and rerun the check; do not call undocumented endpoints. IDs may be pinned in TOML after discovery.

## Visibility

Setup must ask. `users` restricts the collection to explicitly shared members, `organization` shares it with all organization members, and `public` exposes it publicly. For `users`, ask the user to add every required human and agent account manually. Do not silently broaden visibility.

## Legacy migration

Represent old behavior explicitly rather than hardcoding it in the binary. A legacy five-lane profile may map several semantic states onto the same lane:

```toml
[project]
collection = "Existing product"
visibility = "users"

[workflow]
lanes = ["Backlog", "Doing", "Blocked", "Q&A", "Done"]
inbox = "Backlog"
backlog = "Backlog"
ready = "Backlog"
doing = "Doing"
waiting = "Blocked"
blocked = "Blocked"
review = "Q&A"
done = "Done"

[completion]
require_result = false
require_review = false

[history]
require_archive_reference = false

[compatibility]
legacy_cache_path = "/absolute/path/to/pre-0.2/favro-state.json"
allow_unassigned_handoffs = true
```

Pre-0.2 `project.import_legacy_cache` is rejected with a migration-specific error because its old behavior split into two independent settings. Migrate projects one at a time. Preserve collection/board names, lane semantics, watcher assumptions, credential source, and review behavior before adopting generic defaults. `legacy_cache_path` controls only one-time ID-cache import. `allow_unassigned_handoffs` separately preserves checklist/lane handoffs for projects that have not configured participants.
