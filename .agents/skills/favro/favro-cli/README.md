# favro CLI

Companion CLI for the agent-neutral Favro coordination skill. This independent project is not affiliated with or endorsed by Favro AB.

## Build and install

Requires Rust 1.82 or newer.

```bash
cargo install --path . --force
```

Cargo normally installs to `$CARGO_HOME/bin` (or `$HOME/.cargo/bin`). Ensure that directory is on `PATH`. If an existing user-local binary directory appears earlier on `PATH`, install there explicitly or remove the stale copy. For example:

```bash
cargo install --path . --root "$HOME/.local" --force
which -a favro
favro --version
```

`which -a favro` should show the intended installation first; `favro --version` must report 0.2.1 or newer.

When working from the repository root:

```bash
cargo install --path skills/favro/favro-cli --force
```

Authentication uses `FAVRO_EMAIL` and `FAVRO_TOKEN`; `FAVRO_ORG_ID` is required when the account belongs to multiple organizations. A read-only API token supports inspection but cannot create or move cards. Keep secrets in the environment or an ignored env file, never in `.favro/project.toml`.

Run `favro init`, configure the generated project file, then use `favro check`. Upgrade by rerunning the same `cargo install` command with `--force` and verify resolution again with `which -a favro`.

The crate-local `LICENSE` is copied from the repository-root `LICENSE` so Cargo packages include the text. Release review must confirm the two files remain byte-identical.

## Live attachment regression test

Build the CLI with `cargo build --manifest-path skills/favro/favro-cli/Cargo.toml`
from the repository root, then run:

```bash
FAVRO_ENV_FILE=/absolute/path/to/favro.env \
FAVRO_TEST_COLLECTION="Scratch collection" \
FAVRO_TEST_BOARD="Scratch source" \
FAVRO_TEST_MOVE_BOARD="Scratch destination" \
python3 skills/favro/favro-cli/tests/live_attachments.py
```

Both boards must already exist in the scratch collection and have a `Backlog`
column. The test creates its own cards, uploads a small evidence file, exercises
all four description writers plus archive/restore and move-board, and checks
that attachments survive. It also covers repeated edits, duplicate filenames,
Markdown punctuation, checked tasks, and unfinished code fences. Test cards are archived afterward, including on failure;
no existing cards are selected. Credentials alone do not enable writes: the test
skips unless all three scratch settings are supplied. Set `FAVRO_TEST_BINARY` to
test another binary. Uses Python's standard library; no extra packages required.

### Issue #1 investigation (2026-09-09)

Live probes on private scratch cards established the following behavior:

| Description update | Attachment result |
| --- | --- |
| `descriptionFormat=markdown`, description only | Removed |
| Markdown plus `removeAttachments: []` | Removed |
| Markdown plus `attachments` or `addAttachments` arrays | Removed |
| Separate `attachments` / `addAttachments` update after loss | Not restored |
| Plaintext/default format | Preserved, but native checkbox formatting is lost |
| Markdown with ordinary `[name](URL)` links | Removed |
| Markdown with `![name](<fileURL>)` nodes | Preserved, with native checkboxes intact |

The fix uses the last form, in the same PUT. File nodes precede user text so an
unfinished code fence cannot swallow them. Favro deduplicates repeated nodes
for the same file URL; distinct uploads sharing a filename remain distinct.
The published [update-card reference](https://favro.com/developer/#update-a-card)
documents optional attachment removal but does not explain this Markdown
replacement behavior. Version 0.2.1 works around it and verifies the result.

After upgrading a consuming project's CLI to 0.2.1, its issue #1 “attach last”
workaround can be removed. This repository does not contain those downstream
project files.
