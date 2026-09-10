# Upstream provenance: the Favro skill and CLI are vendored, not FAU-authored

Everything in this directory - `SKILL.md`, `references/` and `favro-cli/` - is a **vendored copy
of a subdirectory of another repository**. It is not FAU-authored code, and it is not a git
submodule.

| | |
| --- | --- |
| Upstream repository | https://github.com/MrGaribaldi/favro-agent-skill |
| Upstream path | `skills/favro/` → this directory (`.agents/skills/favro/`) |
| Pinned commit | `e0965df` |
| Crate version | `favro-cli` 0.2.1, binary name `favro` |
| Copied | 9 September 2026 |
| Provenance recorded | 10 September 2026 |
| License | MIT (`favro-cli/LICENSE`) |

The upstream account is the project owner's own, so availability is under project control and no
fork is needed. Commit `e0965df` and the repository were both confirmed reachable on
10 September 2026.

## Why the source is committed rather than fetched

`Dockerfile.agent` does `COPY .agents/skills/favro/favro-cli` followed by
`cargo install --path … --locked --root /usr/local`, so **a clone without this source cannot build
the agent image**. `.dockerignore` draws the same line: the crate source is re-included in the
build context, `favro-cli/target` is excluded. `favro-cli/.gitignore` (FAU's own addition, the one
file here that is not upstream's) keeps the 172 MB build directory out of git.

`SKILL.md` and `references/` are never copied into the image at all - agents read them from the
workspace mount - so they exist only in this repository. That is the second reason they must stay
committed.

## Do not hand-edit anything here

A local edit to `SKILL.md`, `references/` or the crate is silently reverted by the next re-vendor,
and it makes the tree diverge from the pinned sha without any record. FAU's own Favro knowledge -
the facts that are true for this project rather than for the tool - belongs in `/workspace/CLAUDE.md`
under "Favro: what is not in the skill". Tool defects go upstream as an issue, the way the
attachment-loss defect did (#3445, fixed upstream in 0.2.1).

No local patches are known to be applied on top of `e0965df`. That is a statement of intent, not a
machine-verified fact: verifying it means fetching the upstream repository, which happens on the
host rather than from the agent container. To check from the host:

```bash
git clone https://github.com/MrGaribaldi/favro-agent-skill /tmp/fas
git -C /tmp/fas checkout e0965df
diff -r --exclude=target --exclude=.gitignore \
  /tmp/fas/skills/favro <path-to>/.agents/skills/favro
```

Spot-check checksums of the copy as vendored (sha256):

```
LICENSE     e5961fcbc9dee635dca3de05c43190ab6ce4815c417c357ea9003e9c11514c6f
Cargo.lock  b20206a5a399376d8451501a4aa8cf99144e0ed5f48dc754a51bac72e6a347ff
```

## Re-vendoring a new upstream version

Run on the host; the agent container does not perform git operations.

1. Fetch upstream at the new release and note the exact commit sha.
2. Copy `skills/favro/` over this directory. Keep `favro-cli/.gitignore`; never copy `target/`.
3. Confirm `favro-cli/LICENSE` is byte-identical to the upstream repository-root `LICENSE` -
   `favro-cli/README.md` makes that a release-review requirement.
4. Update the table above: commit, version, date.
5. Rebuild the agent image on the host, so `/usr/local/bin/favro` matches the vendored source. A
   `cargo install` from inside the container does **not** survive a container recreate; only the
   image's copy does.
6. Verify `favro --version` and `favro check` in a new session.
