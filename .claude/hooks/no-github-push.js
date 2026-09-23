#!/usr/bin/env node
// PreToolUse(Bash) guard: local commits yes, GitHub no.
//
// agent-box v1.6.0 introduced ALLOW_GIT_WRITE, but that variable is binary -
// "Yes" permits every git and gh write, push included, and there is no value
// meaning "commit but never reach GitHub". FAU needs commits (docker-compose.yml
// sets ALLOW_GIT_WRITE: Yes) without granting the agent access to
// github.com/MrGaribaldi/fau-portalen, so this hook draws the missing line.
//
// Denied: `gh` in any form, `git push`, and the mutating `git remote`
// subcommands. Everything else - commit, add, branch, log, diff, status - passes
// through to git-guard.js, which then applies ALLOW_GIT_WRITE as usual.
//
// Fail-open on anything unparseable, like terraform-guard.js and git-guard.js:
// this is a rule around recognized commands, not a sandbox. The segment split is
// not quote-aware either, so a denied verb inside a quoted argument (`echo "git
// push"`) produces a harmless false deny - the same trade-off git-guard.js
// documents. Rephrase the command rather than weakening the rule. The second half of
// the boundary is that no GH_TOKEN and no SSH private key exist in the
// container, so nothing here can authenticate to GitHub anyway.
//
// Deliberately NOT denied: fetch/pull/clone/ls-remote. They are read-only, they
// cannot authenticate without a credential, and denying them would block reading
// a public upstream (re-vendoring the Favro skill, for one).

const REMOTE_WRITE_SUBCOMMANDS = new Set([
  'add', 'remove', 'rm', 'rename', 'set-url', 'set-head', 'set-branches', 'prune',
]);

// git global options that consume the next token as their value.
const GIT_OPTS_WITH_VALUE = new Set(['-C', '-c', '--git-dir', '--work-tree', '--namespace', '--exec-path']);

// Command wrappers to look through, e.g. `env FOO=1 git push`.
const WRAPPERS = new Set(['env', 'command', 'builtin', 'exec', 'time', 'nice', 'nohup', 'sudo']);

function words(segment) {
  // Not a shell parser: strips quotes so `"git" push` is still recognized.
  const out = [];
  let cur = '';
  let quote = null;
  let started = false;
  for (const ch of segment) {
    if (quote) {
      if (ch === quote) quote = null;
      else cur += ch;
      continue;
    }
    if (ch === '"' || ch === "'") { quote = ch; started = true; continue; }
    if (/\s/.test(ch)) {
      if (started || cur) out.push(cur);
      cur = '';
      started = false;
      continue;
    }
    cur += ch;
  }
  if (started || cur) out.push(cur);
  return out.filter((w) => w.length > 0);
}

function inspect(segment) {
  let w = words(segment);

  // Drop wrappers and leading VAR=value assignments.
  for (;;) {
    if (w.length === 0) return null;
    const head = w[0];
    if (WRAPPERS.has(head) || /^[A-Za-z_][A-Za-z0-9_]*=/.test(head)) { w = w.slice(1); continue; }
    break;
  }

  const base = (w[0] || '').split('/').pop();

  if (base === 'gh') return 'gh';

  if (base !== 'git') return null;

  // Skip git's own global options to find the subcommand.
  let i = 1;
  while (i < w.length && w[i].startsWith('-')) {
    if (GIT_OPTS_WITH_VALUE.has(w[i]) && !w[i].includes('=')) i += 2;
    else i += 1;
  }
  const sub = w[i];
  if (sub === 'push') return 'git push';
  if (sub === 'remote') {
    const next = w[i + 1];
    // Bare `git remote` and `git remote -v` are listings.
    if (next && REMOTE_WRITE_SUBCOMMANDS.has(next)) return `git remote ${next}`;
  }
  return null;
}

let raw = '';
process.stdin.on('data', (c) => { raw += c; });
process.stdin.on('end', () => {
  let command;
  try {
    command = JSON.parse(raw)?.tool_input?.command;
  } catch {
    process.exit(0); // unparseable input: fail open
  }
  if (typeof command !== 'string' || command.length === 0) process.exit(0);

  for (const segment of command.split(/\||&&|\|\||;|\n/)) {
    const found = inspect(segment);
    if (!found) continue;
    const reason =
      `"${found}" is blocked in this deployment: the agent may commit locally, but it has no ` +
      `access to GitHub. Leave the commits in the working tree and let Erik push from the host. ` +
      `See .claude/hooks/no-github-push.js and the ALLOW_GIT_WRITE note in docker-compose.yml.`;
    process.stdout.write(JSON.stringify({
      hookSpecificOutput: {
        hookEventName: 'PreToolUse',
        permissionDecision: 'deny',
        permissionDecisionReason: reason,
      },
    }));
    process.exit(0);
  }
  process.exit(0);
});
