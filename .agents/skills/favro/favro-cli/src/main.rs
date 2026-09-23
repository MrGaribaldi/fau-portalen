//! favro — project-configured Favro coordination client for humans and agents.
//!
//! A TOML project file defines collection visibility, role/workstream boards,
//! workflow lanes, role identities, credential profiles, human participants,
//! completion gates, and existing native shared fields.
//!
//! Auth: HTTP Basic with FAVRO_EMAIL + FAVRO_TOKEN (org id auto-detected,
//! override with FAVRO_ORG_ID). Credentials are read from the environment or
//! a file selected by FAVRO_ENV_FILE, or a local favro.env/.env file.
//!
//! NOTE: the Favro "update a card" endpoint addresses cards by their per-widget
//! `cardId`, NOT the `cardCommonId`. Passing a cardCommonId there returns a
//! misleading `403 Access denied`. `move` therefore resolves the cardId first.

mod attachments;
mod config;

use base64::Engine;
use clap::{Parser, Subcommand};
use config::{ProjectConfig, SharedField};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    io::Write,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

const BASE: &str = "https://favro.com/api/v1";
// Historical comments remain safely editable after role-specific prefixes are adopted.
const LEGACY_BOT_MARK: &str = "🤖";
// Board discovery is always collection-scoped and dynamic; never maintain a hardcoded roster.
const ENV_CANDIDATES: [&str; 2] = ["favro.env", ".env"];
type WidgetIdsCache = HashMap<(String, String), HashSet<String>>;
static WIDGET_IDS_CACHE: OnceLock<Mutex<WidgetIdsCache>> = OnceLock::new();

#[derive(Parser)]
#[command(
    name = "favro",
    version,
    about = "Project-configured Favro coordination for humans and agents."
)]
struct Cli {
    /// Override the project configuration discovered as .favro/project.toml.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Override the configured collection (legacy compatibility).
    #[arg(long, global = true, env = "FAVRO_COLLECTION")]
    collection: Option<String>,
    /// Role key from [roles.<key>]; selects comment identity and credentials.
    #[arg(long, global = true, env = "FAVRO_ROLE")]
    role: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create .favro/project.toml. Prompts for missing project choices.
    Init {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        visibility: Option<String>,
        #[arg(long)]
        interview_granularity: Option<String>,
    },
    /// Validate project configuration and connectivity.
    Check,
    /// List all collections (auth/connectivity check).
    ListCollections,
    /// Ensure the configured collection and every role board/lane exist.
    EnsureProject,
    /// Ensure the selected collection, a board, and its lanes exist.
    EnsureBoard {
        #[arg(long)]
        board: String,
        #[arg(long)]
        refresh: bool,
    },
    /// File a card on a board (default lane: Backlog).
    Add {
        #[arg(long)]
        board: String,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        body: String,
        /// Use a configured body template (`work` or `code`) when --body is empty.
        #[arg(long)]
        template: Option<String>,
        #[arg(long, default_value = "Backlog")]
        lane: String,
        /// Card type tag applied on creation (e.g. EPIC, Task, Bug). Auto-creates the tag.
        #[arg(long = "type")]
        card_type: Option<String>,
    },
    /// Move a card to a lane (takes the cardCommonId printed by add/list).
    Move {
        #[arg(long)]
        card: String,
        #[arg(long)]
        lane: String,
    },
    /// Archive a card (reversible with --undo). Favro has no delete; this hides
    /// the card from the board while keeping it and its history retrievable.
    Archive {
        #[arg(long)]
        card: String,
        /// Why this work is obsolete, rejected, or superseded.
        #[arg(long)]
        reason: Option<String>,
        /// Replacement card/document reference, when applicable.
        #[arg(long)]
        successor: Option<String>,
        /// Durable history/decision document updated before archival.
        #[arg(long)]
        history_ref: Option<String>,
        /// Un-archive instead: bring the card back onto its board.
        #[arg(long)]
        undo: bool,
        /// Per-board cardId to restore when several archived instances exist.
        #[arg(long, requires = "undo")]
        instance: Option<String>,
    },
    /// Move a card to a DIFFERENT board, keeping its comments and history.
    /// Use this to re-target work rather than recreating a card and losing its thread.
    /// Favro cards can legitimately live on several boards at once; this command
    /// assumes one board per card within the selected collection and archives every copy
    /// there outside the destination board.
    MoveBoard {
        #[arg(long)]
        card: String,
        /// Destination board name (must already exist).
        #[arg(long)]
        board: String,
        /// Lane on the destination board (default: Backlog).
        #[arg(long, default_value = "Backlog")]
        lane: String,
    },
    /// Comment on a card using the configured role prefix (legacy: 🤖) unless --raw.
    Comment {
        #[arg(long)]
        card: String,
        #[arg(long)]
        text: String,
        /// Post the text verbatim, without the 🤖 bot marker.
        #[arg(long)]
        raw: bool,
    },
    /// Upload a local file and attach it to a card.
    Attach {
        /// cardCommonId printed by add/list.
        #[arg(long)]
        card: String,
        /// Local file to upload (Favro's limit is 10 MiB).
        #[arg(long)]
        file: PathBuf,
        /// Attachment filename shown in Favro (defaults to the local basename).
        #[arg(long)]
        name: Option<String>,
        /// MIME type override (normally inferred from the filename).
        #[arg(long)]
        mime_type: Option<String>,
        /// Upload first, then remove whichever pre-existing attachment(s) already
        /// carried this filename, so the card ends up with exactly one. The old
        /// copy stays on the card until the new one is confirmed uploaded.
        #[arg(long)]
        replace: bool,
        /// Disambiguates --replace when several existing attachments already share
        /// the new filename: the fileURL (from `get --card <id>`) of the one to
        /// remove. Required in that case; refused otherwise.
        #[arg(long)]
        replace_url: Option<String>,
    },
    /// Remove one attachment from a card, leaving the rest of the description untouched.
    ///
    /// Names are not unique in Favro, so a name matching more than one attachment is
    /// refused unless --url picks one. This unlinks the file from the card; whether
    /// Favro also deletes the underlying storage is outside this command's knowledge.
    Detach {
        #[arg(long)]
        card: String,
        /// Attachment filename to remove.
        #[arg(long)]
        name: String,
        /// Disambiguates when several attachments share --name: the fileURL (from
        /// `get --card <id>`) of the one to remove.
        #[arg(long)]
        url: Option<String>,
    },
    /// Rewrite a comment authored by the selected role, or a legacy 🤖 comment, in place.
    ///
    /// Stale agent comments can mislead later readers. Correcting one in place preserves
    /// the thread while the authorship guard prevents changes to human or other-role text.
    CommentEdit {
        /// The card the comment is on (cardCommonId). Required — the API has no
        /// GET /comments/:id, so the lookup must be scoped to a card.
        #[arg(long)]
        card: String,
        /// commentId, from `favro comments --card <id> --json`.
        #[arg(long)]
        comment: String,
        #[arg(long)]
        text: String,
        /// Write the text verbatim, without re-applying the 🤖 marker. Avoid this on
        /// anything you may later revise: an unmarked comment reads as human-authored
        /// and comment-edit/comment-delete will then refuse to touch it.
        #[arg(long)]
        raw: bool,
    },
    /// Delete a configured-role or legacy 🤖 comment. Same human-safety guard as comment-edit.
    ///
    /// Prefer `comment-edit` — rewriting keeps the thread's shape and says what changed,
    /// where a delete silently removes context someone may have replied to. Use this only
    /// for a comment that should never have been posted at all.
    CommentDelete {
        /// The card the comment is on (cardCommonId). Required, same reason as comment-edit.
        #[arg(long)]
        card: String,
        /// commentId, from `favro comments --card <id> --json`.
        #[arg(long)]
        comment: String,
    },
    /// Print a card as JSON.
    Get {
        #[arg(long)]
        card: String,
    },
    /// Read the comments on a card (oldest first).
    Comments {
        #[arg(long)]
        card: String,
        /// Print the raw comment JSON instead of formatted lines.
        #[arg(long)]
        json: bool,
    },
    /// Set/replace the maintained automation Notes block in the card body.
    SetNotes {
        #[arg(long)]
        card: String,
        #[arg(long)]
        text: String,
    },
    /// Set/replace the "👤 Needs you" checklist PINNED AT THE TOP of a card's description.
    /// This is where every action the HUMAN must take goes — one `--item` per action.
    SetTodo {
        #[arg(long)]
        card: String,
        /// One action the human must take (repeatable). Rendered as `- [ ] <item>`.
        #[arg(long = "item")]
        item: Vec<String>,
        /// Optional explanation, placed directly UNDER the checklist. Keep the items terse
        /// and put any context/why/how here rather than inflating the item text.
        #[arg(long)]
        note: Option<String>,
        /// Configured participant who must act (defaults to project.default_human).
        #[arg(long)]
        participant: Option<String>,
        /// This is an unexpected impediment; use Blocked instead of the normal Waiting lane.
        #[arg(long)]
        blocked: bool,
        /// Remove the block entirely (nothing left for the human to do).
        #[arg(long)]
        clear: bool,
    },
    /// Replace a card's full description/body (for cleanups; set-notes manages only the 🤖 block).
    SetDesc {
        #[arg(long)]
        card: String,
        #[arg(long)]
        text: String,
    },
    /// Set/replace the durable result/evidence block required for completion.
    SetResult {
        #[arg(long)]
        card: String,
        #[arg(long)]
        text: String,
    },
    /// Add/remove tags on a card (card type lives here: EPIC, Task, Bug, ...). Tags auto-create.
    Tag {
        #[arg(long)]
        card: String,
        /// Tag name(s) to add (repeatable).
        #[arg(long = "add")]
        add: Vec<String>,
        /// Tag name(s) to remove (repeatable).
        #[arg(long = "remove")]
        remove: Vec<String>,
    },
    /// Declare a dependency: --card depends on --on (the --on card must finish first). --before flips it.
    Depend {
        #[arg(long)]
        card: String,
        #[arg(long)]
        on: String,
        /// Flip direction: --card must finish before --on (i.e. --card is the prerequisite).
        #[arg(long)]
        before: bool,
    },
    /// List a card's dependencies.
    Deps {
        #[arg(long)]
        card: String,
        /// Print the raw dependency JSON (entries: cardId, cardCommonId, isBefore).
        #[arg(long)]
        json: bool,
    },
    /// List cards on a board (optionally one lane).
    List {
        #[arg(long)]
        board: String,
        #[arg(long)]
        lane: Option<String>,
    },
    /// Legacy alias for the configured review queue.
    QaQueue {
        #[arg(long)]
        board: Vec<String>,
    },
    /// Pull the configured review lane across all project boards.
    ReviewQueue {
        #[arg(long)]
        board: Vec<String>,
    },
    /// Summarize every live card across all project boards.
    Overview,
    /// Add or remove native card assignments by configured participant or role.
    Assign {
        #[arg(long)]
        card: String,
        #[arg(long)]
        add: Vec<String>,
        #[arg(long)]
        remove: Vec<String>,
    },
    /// Set the configured native Priority shared field.
    Priority {
        #[arg(long)]
        card: String,
        #[arg(long)]
        value: String,
        /// Required rationale when priority is Critical.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Set the configured native Complexity rating (1=low, 5=high).
    Complexity {
        #[arg(long)]
        card: String,
        #[arg(long)]
        value: u8,
    },
}

fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("{}", msg.as_ref());
    std::process::exit(1);
}

/// Interpret literal backslash-n / backslash-t in user-supplied text, so callers can pass
/// multi-line card text in a PLAIN double-quoted arg (e.g. "line1\nline2") instead of shell
/// $'...' ANSI-C quoting. The $'...' form does NOT match the `Bash(favro *)` permission
/// allow-rule (embedded newlines make the matcher treat it as a non-plain command), which
/// forces an approval prompt that background agents can't answer -> auto-denied.
fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\t", "\t")
}

/// Favro converts `- [ ]` / `- [x]` markdown lines into NATIVE checkbox tasks, and a GET
/// then reads them back as `☐ …` / `☑ …`. Writing that text back verbatim would store the
/// glyph as literal text and lose both the checkbox and the human's ticks, so every command
/// that rewrites a whole description runs the body through this first — round-tripping the
/// glyphs to markdown, preserving which items are already ticked.
/// Blank lines round-trip the same way: a markdown write stores an empty line as a leading
/// `↵` on the following line, which would pile up as literal glyphs over successive writes.
/// Both are undone here.
fn checkboxes_to_markdown(s: &str) -> String {
    s.lines()
        .map(|line| {
            let t = line.trim_start();
            let indent = &line[..line.len() - t.len()];
            // A stored "↵" is really the blank line that preceded this text.
            let (blank, t) = match t.strip_prefix('↵') {
                Some(rest) => ("\n", rest),
                None => ("", t),
            };
            for (glyph, md) in [("☐ ", "- [ ] "), ("☑ ", "- [x] "), ("☒ ", "- [x] ")] {
                if let Some(rest) = t.strip_prefix(glyph) {
                    return format!("{blank}{indent}{md}{rest}");
                }
            }
            format!("{blank}{indent}{t}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn managed_block_bounds(description: &str, start: &str, end: &str) -> Option<(usize, usize)> {
    let start_at = description.find(start)?;
    let end_at = description[start_at + start.len()..].find(end)? + start_at + start.len();
    Some((start_at, end_at + end.len()))
}

fn remove_managed_block(description: &str, start: &str, end: &str) -> String {
    if let Some((from, through)) = managed_block_bounds(description, start, end) {
        return format!("{}{}", &description[..from], &description[through..]);
    }
    if let Some(from) = description.find(start) {
        // A START without a matching END makes the remainder indeterminate. Treat it as
        // stale managed content rather than preserving it as ordinary body text.
        let mut prefix = description[..from].to_string();
        if let Some(orphan_end) = prefix.find(end) {
            prefix.replace_range(orphan_end..orphan_end + end.len(), "");
        }
        return prefix;
    }
    description.replacen(end, "", 1)
}

fn place_managed_block(
    description: &str,
    start: &str,
    end: &str,
    block: &str,
    at_top: bool,
) -> String {
    if let Some((from, through)) = managed_block_bounds(description, start, end) {
        return format!(
            "{}{}{}",
            &description[..from],
            block,
            &description[through..]
        );
    }
    let rest = remove_managed_block(description, start, end);
    if rest.trim().is_empty() {
        return block.to_string();
    }
    if at_top {
        format!("{block}\n\n{}", rest.trim_start_matches(['\n', '\r']))
    } else {
        format!("{}\n\n{block}", rest.trim_end_matches(['\n', '\r']))
    }
}

/// Every description write must declare markdown, or Favro stores `- [ ]` as literal text.
/// `descriptionFormat` is a QUERY param (plaintext|markdown), default plaintext.
fn md_format() -> [(&'static str, String); 1] {
    [("descriptionFormat", "markdown".to_string())]
}

// ----------------------------------------------------------------- auth / env
fn load_env(project_files: &[PathBuf]) {
    let candidates: Vec<PathBuf> = match std::env::var("FAVRO_ENV_FILE") {
        Ok(path) => vec![PathBuf::from(path)],
        Err(_) => project_files
            .iter()
            .cloned()
            .chain(ENV_CANDIDATES.iter().map(PathBuf::from))
            .collect(),
    };
    for path in candidates {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || !line.contains('=') {
                continue;
            }
            let (k, v) = line.split_once('=').unwrap();
            let key = k.trim();
            if !key.starts_with("FAVRO_") {
                continue;
            }
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if std::env::var(key).is_err() {
                std::env::set_var(key, v);
            }
        }
        return;
    }
    if std::env::var("FAVRO_ENV_FILE").is_ok() {
        die("FAVRO_ENV_FILE does not point to a readable file.");
    }
}

fn creds() -> (String, String) {
    load_env(&[]);
    let email = std::env::var("FAVRO_EMAIL").unwrap_or_default();
    let token = std::env::var("FAVRO_TOKEN").unwrap_or_default();
    if email.is_empty() || token.is_empty() {
        die("ERROR: FAVRO_EMAIL / FAVRO_TOKEN not set (env or a .env file).");
    }
    (email, token)
}

// ----------------------------------------------------------------- state cache
fn state_path() -> PathBuf {
    if let Ok(p) = std::env::var("FAVRO_STATE") {
        return PathBuf::from(p);
    }
    if let Ok(path) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(path).join("favro/state.json");
    }
    if let Ok(path) = std::env::var("HOME") {
        return PathBuf::from(path).join(".cache/favro/state.json");
    }
    std::env::temp_dir().join("favro-state.json")
}

fn read_state() -> Value {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_state(state: &Value) {
    if let Some(parent) = state_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        state_path(),
        serde_json::to_string_pretty(state).unwrap_or_default(),
    );
}

fn collection_cache<'a>(state: &'a Value, org: &str, collection: &str) -> Option<&'a Value> {
    state
        .get("organizations")?
        .get(org)?
        .get("collections")?
        .get(collection)
}

fn collection_cache_mut<'a>(state: &'a mut Value, org: &str, collection: &str) -> &'a mut Value {
    if !state.is_object() {
        *state = json!({});
    }
    let organizations = state
        .as_object_mut()
        .unwrap()
        .entry("organizations")
        .or_insert_with(|| json!({}));
    let organization = organizations
        .as_object_mut()
        .unwrap()
        .entry(org)
        .or_insert_with(|| json!({}));
    let collections = organization
        .as_object_mut()
        .unwrap()
        .entry("collections")
        .or_insert_with(|| json!({}));
    collections
        .as_object_mut()
        .unwrap()
        .entry(collection)
        .or_insert_with(|| json!({}))
}

/// Move the original single-collection cache under its organization and collection.
/// Keep accepting it only for the historical default so upgrades do not mix those board
/// IDs into a newly selected product collection.
fn migrate_legacy_collection_cache(state: &mut Value, legacy: &Value, org: &str, collection: &str) {
    let collection_id = legacy.get("collectionId").cloned();
    let boards = legacy.get("boards").cloned();
    if collection_id.is_none() && boards.is_none() {
        return;
    }
    let cache = collection_cache_mut(state, org, collection);
    if cache.get("collectionId").is_none() {
        if let Some(id) = collection_id {
            cache["collectionId"] = id;
        }
    }
    if cache.get("boards").is_none() {
        if let Some(value) = boards {
            cache["boards"] = value;
        }
    }
}

fn import_legacy_collection_cache(state: &mut Value, org: &str, collection: &str) {
    let Ok(path) = std::env::var("FAVRO_LEGACY_STATE_PATH") else {
        return;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        eprintln!(
            "Warning: configured legacy cache {path:?} is not readable; resolving IDs by name."
        );
        return;
    };
    let Ok(legacy) = serde_json::from_str::<Value>(&contents) else {
        eprintln!(
            "Warning: configured legacy cache {path:?} is not valid JSON; resolving IDs by name."
        );
        return;
    };
    migrate_legacy_collection_cache(state, &legacy, org, collection);
}

fn truncate_utf8(text: &str, max_bytes: usize) -> &str {
    let mut end = text.len().min(max_bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// ----------------------------------------------------------------- HTTP
struct Api {
    auth: String,
    org: String,
}

impl Api {
    fn write_description(&self, before: &Value, description: &str) {
        self.write_description_removing(before, description, &[], &[]);
    }

    /// Same as `write_description`, but omits the attachments whose `fileURL` is in
    /// `omit_urls` when rebuilding the description, and tells the loss-guard that
    /// `expected_removals` (attachment names, one entry per removed attachment) are
    /// intentional rather than data loss. Used by `attach --replace` and `detach`.
    fn write_description_removing(
        &self,
        before: &Value,
        description: &str,
        omit_urls: &[&str],
        expected_removals: &[String],
    ) {
        let description = attachments::preserve_in_markdown_except(
            before,
            &checkboxes_to_markdown(description),
            omit_urls,
        )
        .unwrap_or_else(|error| die(error));
        let card_id = before
            .get("cardId")
            .and_then(Value::as_str)
            .unwrap_or_else(|| die("Card has no cardId; description was not changed."));
        self.request(
            "PUT",
            &format!("/cards/{card_id}"),
            &md_format(),
            Some(json!({"detailedDescription": description})),
            None,
        );
        self.verify_attachments(before, card_id, expected_removals);
    }

    /// Re-read the exact instance, including archived cards, after a mutation.
    /// `expected_removals` lists attachment names this same mutation intentionally
    /// removed (see `attachments::verify`); pass `&[]` when none were.
    fn verify_attachments(&self, before: &Value, card_id: &str, expected_removals: &[String]) {
        let (after, _) = self
            .try_request("GET", &format!("/cards/{card_id}"), &[], None, None)
            .unwrap_or_else(|error| {
                die(format!(
                    "Card {card_id} was updated, but attachment verification failed: {error}"
                ))
            });
        if after.get("cardId").and_then(Value::as_str) != Some(card_id) {
            die(format!("Card {card_id} was updated, but attachment verification returned an invalid card response."));
        }
        attachments::verify(before, &after, expected_removals)
            .unwrap_or_else(|error| die(format!("Card {card_id}: {error}")));
    }

    fn new(org: String) -> Self {
        let (email, token) = creds();
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{email}:{token}"));
        Api {
            auth: format!("Basic {auth}"),
            org,
        }
    }

    /// One request without terminating the process, used when a caller must preserve
    /// and report an earlier successful mutation.
    fn try_request(
        &self,
        method: &str,
        path: &str,
        params: &[(&str, String)],
        body: Option<Value>,
        backend: Option<&str>,
    ) -> Result<(Value, Option<String>), String> {
        let mut url = format!("{BASE}{path}");
        if !params.is_empty() {
            let q: Vec<String> = params
                .iter()
                .map(|(k, v)| format!("{k}={}", percent_encode_query_component(v)))
                .collect();
            url = format!("{url}?{}", q.join("&"));
        }
        let mut req = ureq::request(method, &url)
            .set("Authorization", &self.auth)
            .set(
                "User-Agent",
                concat!("favro-agent-cli/", env!("CARGO_PKG_VERSION")),
            );
        if !self.org.is_empty() {
            req = req.set("organizationId", &self.org);
        }
        if let Some(b) = backend {
            req = req.set("X-Favro-Backend-Identifier", b);
        }
        let result = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match result {
            Ok(resp) => {
                let be = resp
                    .header("X-Favro-Backend-Identifier")
                    .map(|s| s.to_string());
                let v: Value = resp.into_json().unwrap_or_else(|_| json!({}));
                Ok((v, be))
            }
            Err(ureq::Error::Status(code, resp)) => {
                let txt = resp.into_string().unwrap_or_default();
                Err(format!(
                    "Favro API {method} {path} -> {code}: {}",
                    truncate_utf8(&txt, 500)
                ))
            }
            Err(e) => Err(format!("Favro API {method} {path} failed: {e}")),
        }
    }

    /// One request. Returns (json body, X-Favro-Backend-Identifier).
    fn request(
        &self,
        method: &str,
        path: &str,
        params: &[(&str, String)],
        body: Option<Value>,
        backend: Option<&str>,
    ) -> (Value, Option<String>) {
        self.try_request(method, path, params, body, backend)
            .unwrap_or_else(|error| die(error))
    }

    /// Upload a raw file body to Favro's per-card attachment endpoint.
    fn upload(&self, card_id: &str, filename: &str, mime_type: &str, bytes: &[u8]) -> Value {
        let url = format!(
            "{BASE}/cards/{card_id}/attachment?filename={}&mimeType={}",
            percent_encode_query_component(filename),
            percent_encode_query_component(mime_type)
        );
        let mut req = ureq::post(&url)
            .set("Authorization", &self.auth)
            .set(
                "User-Agent",
                concat!("favro-agent-cli/", env!("CARGO_PKG_VERSION")),
            )
            .set("Content-Type", mime_type);
        if !self.org.is_empty() {
            req = req.set("organizationId", &self.org);
        }
        match req.send_bytes(bytes) {
            Ok(resp) => resp.into_json().unwrap_or_else(|e| {
                die(format!("Favro attachment response was not valid JSON: {e}"))
            }),
            Err(ureq::Error::Status(code, resp)) => {
                let txt = resp.into_string().unwrap_or_default();
                die(format!(
                    "Favro API POST /cards/{card_id}/attachment -> {code}: {}",
                    truncate_utf8(&txt, 500)
                ));
            }
            Err(e) => die(format!(
                "Favro API POST /cards/{card_id}/attachment failed: {e}"
            )),
        }
    }

    /// GET, following pagination, returns all entities.
    fn paginate(&self, path: &str, params: &[(&str, String)]) -> Vec<Value> {
        let mut out = Vec::new();
        let mut page = 0u64;
        let mut backend: Option<String> = None;
        let mut request_id: Option<String> = None;
        loop {
            let mut p: Vec<(&str, String)> = params.to_vec();
            p.push(("page", page.to_string()));
            if let Some(rid) = &request_id {
                p.push(("requestId", rid.clone()));
            }
            let (body, be) = self.request("GET", path, &p, None, backend.as_deref());
            if backend.is_none() {
                backend = be;
            }
            if let Some(rid) = body.get("requestId").and_then(|v| v.as_str()) {
                request_id = Some(rid.to_string());
            }
            if let Some(arr) = body.get("entities").and_then(|v| v.as_array()) {
                out.extend(arr.iter().cloned());
            }
            let pages = body.get("pages").and_then(|v| v.as_u64()).unwrap_or(1);
            page += 1;
            if page >= pages {
                break;
            }
        }
        out
    }
}

// ----------------------------------------------------------------- structure
fn resolve_org(state: &mut Value) -> String {
    if let Ok(id) = std::env::var("FAVRO_ORG_ID") {
        return id;
    }
    if let Some(id) = state.get("orgId").and_then(|v| v.as_str()) {
        return id.to_string();
    }
    let api = Api::new(String::new());
    let (body, _) = api.request("GET", "/organizations", &[], None, None);
    let orgs = body
        .get("entities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if orgs.is_empty() {
        die("No Favro organizations found for these credentials.");
    }
    if orgs.len() > 1 {
        let names: Vec<String> = orgs
            .iter()
            .map(|o| {
                format!(
                    "{}={}",
                    o.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                    o.get("organizationId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                )
            })
            .collect();
        die(format!(
            "Multiple orgs; set FAVRO_ORG_ID to one of: {}",
            names.join(", ")
        ));
    }
    let id = orgs[0]
        .get("organizationId")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    state["orgId"] = json!(id);
    write_state(state);
    id
}

fn resolve_collection(api: &Api, state: &mut Value, collection: &str, create: bool) -> String {
    import_legacy_collection_cache(state, &api.org, collection);
    if let Some(id) = collection_cache(state, &api.org, collection)
        .and_then(|value| value.get("collectionId"))
        .and_then(|value| value.as_str())
    {
        return id.to_string();
    }
    let cols = api.paginate("/collections", &[]);
    let found = cols
        .iter()
        .find(|candidate| {
            candidate.get("name").and_then(|value| value.as_str()) == Some(collection)
        })
        .cloned();
    let col = match found {
        Some(value) => value,
        None if create => {
            let (body, _) = api.request(
                "POST",
                "/collections",
                &[],
                Some(json!({"name": collection, "publicSharing": std::env::var("FAVRO_COLLECTION_VISIBILITY").unwrap_or_else(|_| "users".into())})),
                None,
            );
            body
        }
        None => die(format!(
            "Collection {collection:?} does not exist. Run `favro ensure-project` or `favro ensure-board --board <name>` only after confirming creation."
        )),
    };
    let id = col
        .get("collectionId")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    collection_cache_mut(state, &api.org, collection)["collectionId"] = json!(id);
    write_state(state);
    id
}

/// Every live board name in the selected collection, discovered from Favro.
fn all_board_names(collection: &str) -> Vec<String> {
    let mut state = read_state();
    let org = resolve_org(&mut state);
    let api = Api::new(org);
    let collection_id = resolve_collection(&api, &mut state, collection, false);
    let mut names: Vec<String> = api
        .paginate("/widgets", &[("collectionId", collection_id)])
        .iter()
        .filter(|widget| widget.get("archived").and_then(|value| value.as_bool()) != Some(true))
        .filter(|widget| widget.get("type").and_then(Value::as_str) == Some("board"))
        .filter_map(|widget| {
            widget
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

fn collection_widget_ids(api: &Api, collection: &str) -> HashSet<String> {
    let key = (api.org.clone(), collection.to_string());
    let cache = WIDGET_IDS_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(ids) = cache.lock().unwrap().get(&key).cloned() {
        return ids;
    }
    let mut state = read_state();
    let collection_id = resolve_collection(api, &mut state, collection, false);
    let ids: HashSet<String> = api
        .paginate("/widgets", &[("collectionId", collection_id)])
        .into_iter()
        .filter(|widget| widget.get("archived").and_then(Value::as_bool) != Some(true))
        .filter(|widget| widget.get("type").and_then(Value::as_str) == Some("board"))
        .filter_map(|widget| {
            widget
                .get("widgetCommonId")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .collect();
    cache.lock().unwrap().insert(key, ids.clone());
    ids
}

/// Returns (widgetCommonId, {lane: columnId}).
fn resolve_board(
    collection: &str,
    board: &str,
    refresh: bool,
    lanes: &[String],
    create: bool,
) -> (String, serde_json::Map<String, Value>) {
    let mut state = read_state();
    let org = resolve_org(&mut state);
    let api = Api::new(org);
    let collection_id = resolve_collection(&api, &mut state, collection, create);
    if refresh {
        if let Some(object) = collection_cache_mut(&mut state, &api.org, collection).as_object_mut()
        {
            object.remove("boards");
        }
    } else if let Some(info) = collection_cache(&state, &api.org, collection)
        .and_then(|value| value.get("boards"))
        .and_then(|boards| boards.get(board))
    {
        let widget_id = info
            .get("widgetCommonId")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_string();
        let columns = info
            .get("columns")
            .and_then(|value| value.as_object())
            .cloned()
            .unwrap_or_default();
        if lanes.iter().all(|lane| columns.contains_key(lane)) {
            return (widget_id, columns);
        }
    }

    let widgets = api.paginate("/widgets", &[("collectionId", collection_id.clone())]);
    let widget = match widgets
        .iter()
        .find(|widget| {
            widget.get("name").and_then(Value::as_str) == Some(board)
                && widget.get("archived").and_then(Value::as_bool) != Some(true)
                && widget.get("type").and_then(Value::as_str) == Some("board")
        })
        .cloned()
    {
        Some(value) => value,
        None if create => {
            let (body, _) = api.request(
                "POST",
                "/widgets",
                &[],
                Some(json!({
                    "collectionId": collection_id,
                    "name": board,
                    "type": "board",
                    "color": "blue"
                })),
                None,
            );
            body
        }
        None => die(format!(
            "Board {board:?} does not exist in collection {collection:?}. Check the name or create it explicitly with `favro ensure-board --board {board:?}`."
        )),
    };
    let widget_id = widget
        .get("widgetCommonId")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();

    let mut existing = serde_json::Map::new();
    for column in api.paginate("/columns", &[("widgetCommonId", widget_id.clone())]) {
        if let (Some(name), Some(id)) = (
            column.get("name").and_then(|value| value.as_str()),
            column.get("columnId").and_then(|value| value.as_str()),
        ) {
            existing.insert(name.to_string(), json!(id));
        }
    }
    if create {
        for lane in lanes
            .iter()
            .map(String::as_str)
            .chain(std::iter::once("unscheduled"))
        {
            if !existing.contains_key(lane) {
                let (body, _) = api.request(
                    "POST",
                    "/columns",
                    &[],
                    Some(json!({"widgetCommonId": widget_id, "name": lane})),
                    None,
                );
                let id = body
                    .get("columnId")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                existing.insert(lane.to_string(), json!(id));
            }
        }
    }

    let cache = collection_cache_mut(&mut state, &api.org, collection);
    let boards = cache
        .as_object_mut()
        .unwrap()
        .entry("boards")
        .or_insert_with(|| json!({}));
    boards[board] = json!({"widgetCommonId": widget_id, "columns": existing});
    write_state(&state);
    (widget_id, existing)
}

fn enforce_role_emoji_lock(
    config: &ProjectConfig,
    organization: &str,
    collection: &str,
    establish: bool,
) -> Result<(), String> {
    let mut state = read_state();
    let previous = collection_cache(&state, organization, collection)
        .and_then(|cache| cache.get("roleEmojis"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (name, role) in &config.roles {
        if let Some(old) = previous.get(name).and_then(Value::as_str) {
            if old != role.emoji.trim() {
                return Err(format!(
                    "Role {name:?} emoji is locked as {old:?}; project role emojis are immutable."
                ));
            }
        }
    }
    if establish {
        let mut merged = previous;
        for (name, role) in &config.roles {
            merged.insert(name.clone(), json!(role.emoji.trim()));
        }
        collection_cache_mut(&mut state, organization, collection)["roleEmojis"] =
            Value::Object(merged);
        write_state(&state);
    }
    Ok(())
}

fn author_prefix() -> String {
    if let Ok(prefix) = std::env::var("FAVRO_AUTHOR_PREFIX") {
        return prefix;
    }
    if std::env::var("FAVRO_REQUIRE_ROLE_PREFIX").as_deref() == Ok("1") {
        die("This project defines role identities. Select one with --role or FAVRO_ROLE before writing an agent comment.");
    }
    LEGACY_BOT_MARK.into()
}

fn is_agent_comment(text: &str) -> bool {
    let text = text.trim_start();
    text.starts_with(LEGACY_BOT_MARK)
        || std::env::var("FAVRO_AUTHOR_PREFIX")
            .ok()
            .is_some_and(|prefix| !prefix.is_empty() && text.starts_with(&prefix))
}

/// Fetch one comment by id, scoped to its card, and refuse unless the selected role wrote it.
///
/// `card` is REQUIRED because the Favro API has no GET /comments/:id — only a list filtered
/// by cardCommonId. An earlier version took just the commentId and searched every board's
/// every card, which meant hundreds of API calls and a 120 s timeout on the first real use.
/// The caller always has the card anyway: the commentId comes from
/// `favro comments --card <id> --json`.
fn fetch_bot_comment(api: &Api, card: &str, comment_id: &str, verb: &str) -> Value {
    let comments = api.paginate("/comments", &[("cardCommonId", card.to_string())]);
    for c in &comments {
        if c.get("commentId").and_then(|v| v.as_str()) != Some(comment_id) {
            continue;
        }
        let text = c.get("comment").and_then(|v| v.as_str()).unwrap_or("");
        if !is_agent_comment(text) {
            die(format!(
                "Refusing to {verb} comment {comment_id}: it does not start with the selected role prefix or legacy {LEGACY_BOT_MARK} marker. \
                 so as far as we can tell a human wrote it. Automation has overwritten user text before. If it really must change, do it in the Favro web UI.\n\
                 \n  NOTE: an automation-authored comment posted with `--raw` also has no marker and lands here. \
                 That is why agents should not use --raw for anything they may later revise.\n\
                 \n  text: {}",
                truncate_utf8(text, 200)
            ));
        }
        return c.clone();
    }
    die(format!(
        "Comment {comment_id} not found on card {card} ({} comment(s) there). Get the id from \
         `favro comments --card {card} --json` — it is the commentId field, not cardCommonId, \
         and it must be a comment on THIS card.",
        comments.len()
    ))
}

fn api_handle() -> Api {
    let mut state = read_state();
    let org = resolve_org(&mut state);
    Api::new(org)
}

fn select_card_instance(
    cards: Vec<Value>,
    selected_widgets: &HashSet<String>,
    card_common_id: &str,
    collection: &str,
) -> Result<Value, String> {
    let cards: Vec<Value> = cards
        .into_iter()
        .filter(|card| {
            card.get("widgetCommonId")
                .and_then(Value::as_str)
                .is_some_and(|widget| selected_widgets.contains(widget))
        })
        .collect();
    let active: Vec<&Value> = cards
        .iter()
        .filter(|card| card.get("archived").and_then(Value::as_bool) != Some(true))
        .collect();
    match active.as_slice() {
        [card] => Ok((*card).clone()),
        [] if cards.len() == 1 => Ok(cards[0].clone()),
        [] if cards.is_empty() => Err(format!(
            "Card {card_common_id} was not found in collection {collection:?}."
        )),
        [] => Err(format!(
            "Card {card_common_id} has multiple archived instances in collection {collection:?}; unarchive it in Favro or specify a unique active instance."
        )),
        _ => Err(format!(
            "Card {card_common_id} has multiple active instances in collection {collection:?}; resolve the duplicate board placement before mutating it."
        )),
    }
}

fn find_card(api: &Api, collection: &str, card_common_id: &str) -> Value {
    let selected_widgets = collection_widget_ids(api, collection);
    let cards = api.paginate("/cards", &[("cardCommonId", card_common_id.to_string())]);
    select_card_instance(cards, &selected_widgets, card_common_id, collection)
        .unwrap_or_else(|error| die(error))
}

/// Get a fresh snapshot of the exact instance, rather than relying on paginated
/// list data for a destructive whole-description update.
fn find_description_card(api: &Api, collection: &str, card: &str) -> Value {
    let selected = find_card(api, collection, card);
    let card_id = selected
        .get("cardId")
        .and_then(Value::as_str)
        .unwrap_or_else(|| die("Card has no cardId; description was not changed."));
    // Plaintext excludes file image nodes. write_description reconstructs those
    // from attachments, keeping them out of managed notes/result/TODO blocks.
    let (snapshot, _) = api.request("GET", &format!("/cards/{card_id}"), &[], None, None);
    snapshot
}

fn find_archive_target(
    api: &Api,
    collection: &str,
    card_common_id: &str,
    undo: bool,
    instance: Option<&str>,
) -> Value {
    if !undo {
        return find_card(api, collection, card_common_id);
    }
    let selected_widgets = collection_widget_ids(api, collection);
    let cards: Vec<Value> = api
        .paginate("/cards", &[("cardCommonId", card_common_id.to_string())])
        .into_iter()
        .filter(|card| {
            card.get("widgetCommonId")
                .and_then(Value::as_str)
                .is_some_and(|widget| selected_widgets.contains(widget))
        })
        .collect();
    if let Some(card_id) = instance {
        let card = cards
            .into_iter()
            .find(|candidate| candidate.get("cardId").and_then(Value::as_str) == Some(card_id))
            .unwrap_or_else(|| {
                die(format!(
                    "Archived instance {card_id:?} was not found for card {card_common_id} in collection {collection:?}."
                ))
            });
        if card.get("archived").and_then(Value::as_bool) != Some(true) {
            die(format!("Card instance {card_id:?} is already active."));
        }
        return card;
    }
    let archived: Vec<Value> = cards
        .iter()
        .filter(|card| card.get("archived").and_then(Value::as_bool) == Some(true))
        .cloned()
        .collect();
    match archived.as_slice() {
        [card] => card.clone(),
        [] if cards
            .iter()
            .any(|card| card.get("archived").and_then(Value::as_bool) != Some(true)) =>
        {
            die(format!("Card {card_common_id} is already active."))
        }
        [] => die(format!(
            "Card {card_common_id} has no archived instance in collection {collection:?}."
        )),
        _ => {
            let ids = archived
                .iter()
                .filter_map(|card| card.get("cardId").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            die(format!(
                "Card {card_common_id} has several archived instances. Choose one with `favro archive --card {card_common_id} --undo --instance <cardId>`. Candidates: {ids}"
            ))
        }
    }
}

fn percent_encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn infer_mime_type(filename: &str) -> &'static str {
    let extension = PathBuf::from(filename)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("md" | "markdown") => "text/markdown",
        Some("txt" | "log") => "text/plain",
        Some("json") => "application/json",
        Some("csv") => "text/csv",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("zip") => "application/zip",
        _ => "application/octet-stream",
    }
}

fn prompt(label: &str, default: &str) -> String {
    print!("{label} [{default}]: ");
    std::io::stdout()
        .flush()
        .unwrap_or_else(|e| die(format!("Cannot write prompt: {e}")));
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .unwrap_or_else(|e| die(format!("Cannot read setup answer: {e}")));
    let answer = answer.trim();
    if answer.is_empty() {
        default.to_string()
    } else {
        answer.to_string()
    }
}

fn suggested_project_name() -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("project"));
    let mut dir = cwd.as_path();
    loop {
        if dir.join(".git").exists() {
            return dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("project")
                .to_string();
        }
        let Some(parent) = dir.parent() else { break };
        dir = parent;
    }
    cwd.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string()
}

fn memorable_name() -> String {
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as usize)
        .unwrap_or(0);
    let a = ["amber", "brisk", "calm", "clever", "silver", "steady"];
    let b = ["badger", "falcon", "horse", "otter", "raven", "turtle"];
    let c = [
        "battery", "bridge", "compass", "harbor", "lantern", "staple",
    ];
    format!(
        "{}-{}-{}",
        a[seed % a.len()],
        b[(seed / 7) % b.len()],
        c[(seed / 41) % c.len()]
    )
}

fn init_project(
    explicit: Option<&Path>,
    name: Option<String>,
    visibility: Option<String>,
    granularity: Option<String>,
) {
    let path = explicit.map(Path::to_path_buf).unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(".favro/project.toml")
    });
    if path.exists() {
        die(format!(
            "Refusing to overwrite existing project config {}",
            path.display()
        ));
    }
    let suggested = suggested_project_name();
    let random = memorable_name();
    let name = name.unwrap_or_else(|| {
        println!("Suggested from repository/directory: {suggested}");
        println!("Generated alternative: {random}");
        let answer = prompt(
            "Collection name (enter 'random' for generated alternative)",
            &suggested,
        );
        if answer == "random" {
            random
        } else {
            answer
        }
    });
    let visibility =
        visibility.unwrap_or_else(|| prompt("Visibility: users, organization, or public", "users"));
    if !["users", "organization", "public"].contains(&visibility.as_str()) {
        die("Visibility must be users, organization, or public.");
    }
    let granularity =
        granularity.unwrap_or_else(|| prompt("Interview cards: topic or question", "topic"));
    if !["topic", "question"].contains(&granularity.as_str()) {
        die("Interview granularity must be topic or question.");
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| die(format!("Cannot create {}: {e}", parent.display())));
    }
    std::fs::write(&path, config::starter(&name, &visibility, &granularity))
        .unwrap_or_else(|e| die(format!("Cannot write {}: {e}", path.display())));
    println!("Created {}", path.display());
    if visibility == "users" {
        println!("The collection will be restricted to explicitly shared users. Add every required human/agent Favro account before assigning cards to them.");
    }
}

fn assignment_id(config: &ProjectConfig, name: &str) -> Result<String, String> {
    if let Some(person) = config.participants.get(name) {
        if person.user_id.trim().is_empty() {
            return Err(format!("participant {name:?} has no user_id"));
        }
        return Ok(person.user_id.clone());
    }
    if let Some(role) = config.roles.get(name) {
        let auth_name = role
            .auth
            .as_ref()
            .ok_or_else(|| format!("role {name:?} has no auth profile"))?;
        let user_id = config
            .auth
            .get(auth_name)
            .and_then(|a| a.user_id.clone())
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| format!("auth profile {auth_name:?} has no user_id"))?;
        return Ok(user_id);
    }
    Err(format!("Unknown participant or role {name:?}"))
}

fn normalized_field_type(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn find_shared_field(api: &Api, spec: &SharedField) -> Value {
    api.paginate("/customfields", &[])
        .into_iter()
        .filter(|field| field.get("widgetCommonId").is_none_or(Value::is_null))
        .find(|field| {
            spec.id.as_deref().is_some_and(|id| field.get("customFieldId").and_then(Value::as_str) == Some(id))
                || field.get("name").and_then(Value::as_str) == Some(spec.name.as_str())
        })
        .unwrap_or_else(|| die(format!(
            "Organization-shared field {:?} is missing. Create it manually in Favro, then rerun `favro check`.", spec.name
        )))
}

fn shared_field_id(field: &Value) -> String {
    field
        .get("customFieldId")
        .and_then(Value::as_str)
        .unwrap_or_else(|| die("Favro returned a custom field without customFieldId."))
        .to_string()
}

fn field_item_id(field: &Value, requested: &str) -> String {
    for key in ["customFieldItems", "items", "values"] {
        if let Some(items) = field.get(key).and_then(Value::as_array) {
            if let Some(item) = items.iter().find(|item| {
                ["name", "value", "label"].iter().any(|key| {
                    item.get(key)
                        .and_then(Value::as_str)
                        .is_some_and(|value| value.eq_ignore_ascii_case(requested))
                })
            }) {
                for key in ["customFieldItemId", "itemId", "id"] {
                    if let Some(id) = item.get(key).and_then(Value::as_str) {
                        return id.to_string();
                    }
                }
            }
        }
    }
    die(format!(
        "Value {requested:?} is not an option in shared field {:?}.",
        field.get("name").and_then(Value::as_str).unwrap_or("?")
    ))
}

// ----------------------------------------------------------------- commands
fn main() {
    let cli = Cli::parse();
    if let Cmd::Init {
        name,
        visibility,
        interview_granularity,
    } = &cli.cmd
    {
        init_project(
            cli.config.as_deref(),
            name.clone(),
            visibility.clone(),
            interview_granularity.clone(),
        );
        return;
    }

    let project = ProjectConfig::load(cli.config.as_deref()).unwrap_or_else(|e| die(e));
    let project_env_files = project
        .as_ref()
        .map(ProjectConfig::credential_files)
        .unwrap_or_default();
    load_env(&project_env_files);
    if let Some(config) = &project {
        let auth_role = cli.role.as_deref().or_else(|| {
            if matches!(&cli.cmd, Cmd::Check | Cmd::ListCollections) {
                config.roles.keys().min().map(String::as_str)
            } else {
                None
            }
        });
        config.apply_auth(auth_role).unwrap_or_else(|e| die(e));
        std::env::set_var("FAVRO_COLLECTION_VISIBILITY", &config.project.visibility);
        if let Some(path) = &config.compatibility.legacy_cache_path {
            std::env::set_var("FAVRO_LEGACY_STATE_PATH", path);
        }
        if !config.roles.is_empty() {
            std::env::set_var("FAVRO_REQUIRE_ROLE_PREFIX", "1");
        }
        if let Some(prefix) = config
            .author_prefix(cli.role.as_deref())
            .unwrap_or_else(|e| die(e))
        {
            std::env::set_var("FAVRO_AUTHOR_PREFIX", prefix);
        }
    }
    if matches!(&cli.cmd, Cmd::ListCollections) {
        let api = api_handle();
        for item in api.paginate("/collections", &[]) {
            println!(
                "{}  {}",
                item.get("collectionId")
                    .and_then(Value::as_str)
                    .unwrap_or("?"),
                item.get("name").and_then(Value::as_str).unwrap_or("?")
            );
        }
        return;
    }
    let collection = cli.collection.clone()
        .or_else(|| project.as_ref().map(|c| c.project.collection.clone()))
        .unwrap_or_else(|| die("No Favro project is configured. Ask the user to choose a project name, then run `favro init --name <name>`."));
    if collection.trim().is_empty() {
        die("Favro collection name must not be empty.");
    }
    let lanes = project
        .as_ref()
        .map(|c| c.workflow.lanes.clone())
        .unwrap_or_else(|| {
            config::GENERIC_LANES
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    let default_lane = project
        .as_ref()
        .map(|c| c.workflow.backlog.clone())
        .unwrap_or_else(|| "Backlog".into());
    let review_lane = project
        .as_ref()
        .map(|c| c.workflow.review.clone())
        .unwrap_or_else(|| "Review".into());
    match cli.cmd {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Check => {
            let config = project
                .as_ref()
                .unwrap_or_else(|| die("No .favro/project.toml found. Run `favro init`."));
            for (name, profile) in &config.auth {
                for (label, env_name) in [
                    ("email", profile.email_env.as_str()),
                    ("token", profile.token_env.as_str()),
                ] {
                    if std::env::var(env_name).is_err() {
                        die(format!(
                            "Auth profile {name:?} {label} variable {env_name:?} is not set."
                        ));
                    }
                }
                if let Some(env_name) = &profile.organization_id_env {
                    if std::env::var(env_name).is_err() {
                        die(format!(
                            "Auth profile {name:?} organization variable {env_name:?} is not set."
                        ));
                    }
                }
            }
            let api = api_handle();
            enforce_role_emoji_lock(config, &api.org, &collection, false)
                .unwrap_or_else(|error| die(error));
            let collections = api.paginate("/collections", &[]);
            let configured_collection = collections
                .iter()
                .find(|c| c.get("name").and_then(Value::as_str) == Some(collection.as_str()));
            if let Some(found) = configured_collection {
                if let Some(actual) = found.get("publicSharing").and_then(Value::as_str) {
                    if actual != config.project.visibility {
                        die(format!("Collection visibility is {actual:?}, but project.toml requires {:?}. Change it deliberately in Favro or update the config.", config.project.visibility));
                    }
                }
                let collection_id = found
                    .get("collectionId")
                    .and_then(Value::as_str)
                    .unwrap_or_else(|| die("Favro returned a collection without collectionId."));
                let board_names: HashSet<String> = api
                    .paginate("/widgets", &[("collectionId", collection_id.to_string())])
                    .into_iter()
                    .filter(|widget| {
                        widget.get("archived").and_then(Value::as_bool) != Some(true)
                            && widget.get("type").and_then(Value::as_str) == Some("board")
                    })
                    .filter_map(|widget| {
                        widget
                            .get("name")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect();
                for (name, role) in &config.roles {
                    if !board_names.contains(&role.board) {
                        die(format!(
                            "Role {name:?} board {:?} does not exist in collection {:?}.",
                            role.board, collection
                        ));
                    }
                }
            } else {
                println!(
                    "Collection {:?} is configured but has not been created yet.",
                    collection
                );
            }
            let user_ids: HashSet<String> = api
                .paginate("/users", &[])
                .into_iter()
                .filter_map(|user| {
                    user.get("userId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect();
            for (name, participant) in &config.participants {
                if !user_ids.contains(&participant.user_id) {
                    die(format!(
                        "Participant {name:?} user_id does not resolve to a Favro user."
                    ));
                }
            }
            for (name, profile) in &config.auth {
                if let Some(user_id) = &profile.user_id {
                    if !user_ids.contains(user_id) {
                        die(format!(
                            "Auth profile {name:?} user_id does not resolve to a Favro user."
                        ));
                    }
                }
            }
            for (label, spec, allowed) in [
                (
                    "priority",
                    config.fields.priority.as_ref(),
                    &["status", "multipleselect"][..],
                ),
                (
                    "complexity",
                    config.fields.complexity.as_ref(),
                    &["rating"][..],
                ),
            ] {
                if let Some(spec) = spec {
                    let field = find_shared_field(&api, spec);
                    let kind = field.get("type").and_then(Value::as_str).unwrap_or("?");
                    if !allowed.contains(&normalized_field_type(kind).as_str()) {
                        die(format!("Configured {label} field {:?} has type {kind:?}; expected {allowed:?}.", spec.name));
                    }
                    if label == "priority" {
                        for required in ["Critical", "High", "Normal", "Low"] {
                            field_item_id(&field, required);
                        }
                    }
                    println!(
                        "{label}: {:?} ({kind}, {})",
                        spec.name,
                        shared_field_id(&field)
                    );
                }
            }
            println!(
                "Configuration OK: {}",
                config
                    .source
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
        }
        Cmd::ListCollections => unreachable!(),
        Cmd::EnsureProject => {
            let config = project
                .as_ref()
                .unwrap_or_else(|| die("ensure-project requires .favro/project.toml."));
            if config.roles.is_empty() {
                die("No roles are configured; add [roles.<name>] entries first.");
            }
            let api = api_handle();
            enforce_role_emoji_lock(config, &api.org, &collection, true)
                .unwrap_or_else(|error| die(error));
            for role in config.roles.values() {
                let (widget, _) = resolve_board(&collection, &role.board, false, &lanes, true);
                println!("Ensured {:?} ({widget})", role.board);
            }
        }
        Cmd::EnsureBoard { board, refresh } => {
            if project.is_none() {
                die("ensure-board requires .favro/project.toml so collection visibility is explicit.");
            }
            let (wid, cols) = resolve_board(&collection, &board, refresh, &lanes, true);
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"widgetCommonId": wid, "columns": cols}))
                    .unwrap()
            );
        }
        Cmd::Add {
            board,
            title,
            body,
            template,
            lane,
            card_type,
        } => {
            let body = if body.is_empty() {
                match template.as_deref() {
                    Some("work") => project
                        .as_ref()
                        .map(|c| c.templates.work.clone())
                        .unwrap_or_default(),
                    Some("code") => project
                        .as_ref()
                        .map(|c| c.templates.code.clone())
                        .unwrap_or_default(),
                    Some(other) => die(format!("Unknown template {other:?}; use work or code.")),
                    None => body,
                }
            } else if template.is_some() {
                die("Use either --body or --template, not both.");
            } else {
                body
            };
            let body = unescape(&body);
            let lane = if lane == "Backlog" && !lanes.contains(&lane) {
                default_lane.clone()
            } else {
                lane
            };
            let (wid, cols) = resolve_board(&collection, &board, false, &lanes, false);
            let col = cols.get(&lane).and_then(|v| v.as_str()).unwrap_or_else(|| {
                die(format!(
                    "Unknown lane {lane:?}; configured lanes are {lanes:?}"
                ))
            });
            let assignment_ids: Vec<String> = project
                .as_ref()
                .and_then(|config| config.role(cli.role.as_deref()).unwrap_or_else(|e| die(e)))
                .and_then(|(_, role)| {
                    role.auth
                        .as_ref()
                        .and_then(|name| project.as_ref().unwrap().auth.get(name))
                })
                .and_then(|auth| auth.user_id.clone())
                .into_iter()
                .collect();
            let api = api_handle();
            // Validate configured metadata before creating anything, so a missing shared
            // field cannot produce an unreported card.
            let default_priority = project
                .as_ref()
                .and_then(|config| config.fields.priority.as_ref())
                .map(|spec| {
                    let field = find_shared_field(&api, spec);
                    (shared_field_id(&field), field_item_id(&field, "Normal"))
                });
            let (resp, _) = api.request(
                "POST",
                "/cards",
                &md_format(),
                Some(json!({
                    "name": unescape(&title),
                    "detailedDescription": body,
                    "widgetCommonId": wid,
                    "columnId": col,
                    "assignmentIds": assignment_ids,
                })),
                None,
            );
            let card = resp
                .get("entities")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or(resp);
            println!(
                "Created on {board}/{lane}: #{} {}  [{}]",
                card.get("sequentialId")
                    .map(|v| v.to_string())
                    .unwrap_or("?".into()),
                card.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                card.get("cardCommonId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
            );
            if let Some(cid) = card.get("cardId").and_then(Value::as_str) {
                if let Some(t) = card_type {
                    if let Err(error) = api.try_request(
                        "PUT",
                        &format!("/cards/{cid}"),
                        &[],
                        Some(json!({"addTags": [t]})),
                        None,
                    ) {
                        eprintln!(
                            "Warning: card was created, but its type tag was not applied: {error}"
                        );
                    }
                }
                if let Some((field_id, normal)) = default_priority {
                    if let Err(error) = api.try_request(
                        "PUT",
                        &format!("/cards/{cid}"),
                        &[],
                        Some(json!({
                            "customFields": [{"customFieldId": field_id, "value": [normal]}]
                        })),
                        None,
                    ) {
                        eprintln!("Warning: card was created, but default Priority was not applied: {error}");
                    }
                }
            } else {
                eprintln!("Warning: Favro created the card but returned no cardId; optional metadata was not applied.");
            }
        }
        Cmd::Move { card, lane } => {
            if !lanes.contains(&lane) {
                die(format!(
                    "Unknown lane {lane:?}; configured lanes are {lanes:?}"
                ));
            }
            let api = api_handle();
            let c = find_card(&api, &collection, &card);
            if project
                .as_ref()
                .is_some_and(|cfg| lane == cfg.workflow.done)
            {
                let cfg = project.as_ref().unwrap();
                if cfg.completion.require_result
                    && !c
                        .get("detailedDescription")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .contains("<!-- RESULT -->")
                {
                    die("Done requires durable result/evidence. Run `favro set-result --card <id> --text <evidence>` first.");
                }
                if cfg.completion.require_review {
                    let current_column = c.get("columnId").and_then(Value::as_str).unwrap_or("");
                    let wid = c
                        .get("widgetCommonId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let reviewed = api
                        .paginate("/columns", &[("widgetCommonId", wid)])
                        .into_iter()
                        .any(|column| {
                            column.get("columnId").and_then(Value::as_str) == Some(current_column)
                                && column.get("name").and_then(Value::as_str)
                                    == Some(cfg.workflow.review.as_str())
                        });
                    if !reviewed {
                        die(format!(
                            "Done requires the card to pass through {:?} first.",
                            cfg.workflow.review
                        ));
                    }
                }
            }
            // Update-a-card addresses by the per-widget cardId, NOT cardCommonId.
            let card_id = c
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let wid = c
                .get("widgetCommonId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let mut col = None;
            for cc in api.paginate("/columns", &[("widgetCommonId", wid.clone())]) {
                if cc.get("name").and_then(|v| v.as_str()) == Some(&lane) {
                    col = cc
                        .get("columnId")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
            let col =
                col.unwrap_or_else(|| die(format!("Board for this card has no lane {lane:?}.")));
            api.request(
                "PUT",
                &format!("/cards/{card_id}"),
                &[],
                Some(json!({"widgetCommonId": wid, "columnId": col})),
                None,
            );
            println!("Moved {card} -> {lane}");
        }
        Cmd::Archive {
            card,
            reason,
            successor,
            history_ref,
            undo,
            instance,
        } => {
            let api = api_handle();
            let c = find_archive_target(&api, &collection, &card, undo, instance.as_deref());
            if !undo {
                if project
                    .as_ref()
                    .is_some_and(|cfg| cfg.history.require_archive_reference)
                {
                    if reason.as_deref().is_none_or(str::is_empty) {
                        die("Archiving requires --reason in this project.");
                    }
                    if history_ref.as_deref().is_none_or(str::is_empty) {
                        let hint = project
                            .as_ref()
                            .and_then(|cfg| cfg.history.path.as_deref())
                            .unwrap_or("the configured history/decision document");
                        die(format!(
                            "Update {hint}, then pass --history-ref with its path or link."
                        ));
                    }
                }
                if let Some(reason) = &reason {
                    let mut message = format!("Archival rationale: {}", unescape(reason));
                    if let Some(value) = &successor {
                        message.push_str(&format!("\nSuccessor: {}", unescape(value)));
                    }
                    if let Some(value) = &history_ref {
                        message.push_str(&format!("\nHistory: {}", unescape(value)));
                    }
                    api.request("POST", "/comments", &[], Some(json!({
                        "cardCommonId": card, "comment": format!("{} {message}", author_prefix())
                    })), None);
                }
            }
            // Update-a-card addresses by the per-widget cardId, NOT cardCommonId.
            let card_id = c
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let name = c
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let seq = c
                .get("sequentialId")
                .map(|v| v.to_string())
                .unwrap_or("?".into());
            // Favro's update-a-card parameter is `archive`; the field it sets and
            // reports back is `archived`. Sending `archived` is silently accepted
            // and does nothing, so ALWAYS verify rather than trusting the 200.
            api.request(
                "PUT",
                &format!("/cards/{card_id}"),
                &[],
                Some(json!({"archive": !undo})),
                None,
            );
            let after = api
                .paginate("/cards", &[("cardCommonId", card.clone())])
                .into_iter()
                .find(|candidate| candidate.get("cardId").and_then(Value::as_str) == Some(&card_id))
                .unwrap_or_else(|| {
                    die(format!(
                        "FAILED: Favro did not return the updated card instance {card_id}."
                    ))
                });
            attachments::verify(&c, &after, &[])
                .unwrap_or_else(|error| die(format!("Card {card_id}: {error}")));
            let now = after
                .get("archived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if now == undo {
                die(format!(
                    "FAILED: #{seq} is still archived={now} after the request. Nothing was changed."
                ));
            }
            println!(
                "{} #{seq} {name}  [{card}]",
                if undo { "Un-archived" } else { "Archived" }
            );
        }
        Cmd::MoveBoard { card, board, lane } => {
            if !lanes.contains(&lane) {
                die(format!(
                    "Unknown lane {lane:?}; configured lanes are {lanes:?}"
                ));
            }
            let api = api_handle();
            let (wid, cols) = resolve_board(&collection, &board, false, &lanes, false);
            let col = cols
                .get(lane.as_str())
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| die(format!("Board {board:?} has no lane {lane:?}.")))
                .to_string();
            let c = find_card(&api, &collection, &card);
            let card_id = c
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let seq = c
                .get("sequentialId")
                .map(|v| v.to_string())
                .unwrap_or("?".into());
            api.request(
                "PUT",
                &format!("/cards/{card_id}"),
                &[],
                Some(json!({"widgetCommonId": wid, "columnId": col})),
                None,
            );
            // A Favro card is a cardCommonId with one cardId PER BOARD, so the PUT
            // above ADDS it to the destination rather than moving it — the source
            // instance survives and keeps showing on the old board. Archive that
            // instance so the card reads as moved. Comments live on the
            // cardCommonId, so nothing is lost either way.
            let selected_widgets = collection_widget_ids(&api, &collection);
            let instances = api.paginate("/cards", &[("cardCommonId", card.clone())]);
            let target = instances.iter().find(|instance| {
                instance.get("widgetCommonId").and_then(Value::as_str) == Some(wid.as_str())
                    && instance.get("archived").and_then(Value::as_bool) != Some(true)
            }).unwrap_or_else(|| die(format!(
                "FAILED: #{seq} did not appear on {board:?}. Left the original alone; nothing was archived."
            )));
            let target_id = target
                .get("cardId")
                .and_then(Value::as_str)
                .unwrap_or_else(|| die("Destination card has no cardId; nothing was archived."));
            // Verify the destination before retiring any source instance.
            api.verify_attachments(&c, target_id, &[]);
            let mut retired = 0usize;
            for inst in &instances {
                let iw = inst
                    .get("widgetCommonId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let iid = inst.get("cardId").and_then(|v| v.as_str()).unwrap_or("");
                let archived = inst
                    .get("archived")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if iw != wid && selected_widgets.contains(iw) && !archived {
                    api.request(
                        "PUT",
                        &format!("/cards/{iid}"),
                        &[],
                        Some(json!({"archive": true})),
                        None,
                    );
                    api.verify_attachments(inst, iid, &[]);
                    retired += 1;
                }
            }
            api.verify_attachments(&c, target_id, &[]);
            println!(
                "Moved #{seq} [{card}] -> {board}/{lane}{}",
                if retired > 0 {
                    " (archived the old board's copy)"
                } else {
                    ""
                }
            );
        }
        Cmd::Comment { card, text, raw } => {
            let api = api_handle();
            let text = unescape(&text);
            let body = if raw {
                text
            } else {
                format!("{} {text}", author_prefix())
            };
            api.request(
                "POST",
                "/comments",
                &[],
                Some(json!({"cardCommonId": card, "comment": body})),
                None,
            );
            println!("Commented on {card}");
        }
        Cmd::Attach {
            card,
            file,
            name,
            mime_type,
            replace,
            replace_url,
        } => {
            if replace_url.is_some() && !replace {
                die("--replace-url requires --replace.");
            }
            const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;
            let metadata = std::fs::metadata(&file)
                .unwrap_or_else(|e| die(format!("Cannot inspect {}: {e}", file.display())));
            if !metadata.is_file() {
                die(format!(
                    "Attachment path is not a regular file: {}",
                    file.display()
                ));
            }
            if metadata.len() > MAX_ATTACHMENT_BYTES {
                die(format!(
                    "Attachment is {} bytes; Favro's maximum is {MAX_ATTACHMENT_BYTES} bytes (10 MiB).",
                    metadata.len()
                ));
            }
            let filename = name.unwrap_or_else(|| {
                file.file_name()
                    .and_then(|value| value.to_str())
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        die(format!("Cannot derive a filename from {}", file.display()))
                    })
            });
            if filename.is_empty() || filename.contains('/') || filename.contains('\\') {
                die("--name must be a non-empty filename, not a path.");
            }
            let mime_type = mime_type.unwrap_or_else(|| infer_mime_type(&filename).to_string());
            let bytes = std::fs::read(&file)
                .unwrap_or_else(|e| die(format!("Cannot read {}: {e}", file.display())));
            let api = api_handle();
            let existing = find_card(&api, &collection, &card);
            let card_id = existing
                .get("cardId")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| die(format!("Card {card} has no cardId.")));
            // Resolve the attachment --replace will remove BEFORE uploading, so an
            // ambiguous request refuses without ever touching the card.
            let same_name: Vec<&Value> = existing
                .get("attachments")
                .and_then(Value::as_array)
                .map(|attachments| {
                    attachments
                        .iter()
                        .filter(|attachment| {
                            attachment.get("name").and_then(Value::as_str)
                                == Some(filename.as_str())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let outgoing_url: Option<String> = if !replace {
                None
            } else {
                match (same_name.as_slice(), &replace_url) {
                    ([], _) => None,
                    ([one], None) => Some(
                        one.get("fileURL")
                            .and_then(Value::as_str)
                            .unwrap_or_else(|| {
                                die(format!(
                                    "Cannot replace {filename:?}: the existing attachment has no fileURL."
                                ))
                            })
                            .to_string(),
                    ),
                    (_, Some(url)) => {
                        if !same_name.iter().any(|attachment| {
                            attachment.get("fileURL").and_then(Value::as_str) == Some(url.as_str())
                        }) {
                            die(format!(
                                "Card {card} has no attachment named {filename:?} with fileURL {url:?}."
                            ));
                        }
                        Some(url.clone())
                    }
                    (_, None) => die(format!(
                        "Card {card} already has {} attachments named {filename:?}; pass --replace-url <fileURL> to pick one (see `favro get --card {card}`).",
                        same_name.len()
                    )),
                }
            };
            let response = api.upload(card_id, &filename, &mime_type, &bytes);
            if response.get("name").and_then(|value| value.as_str()) != Some(&filename) {
                die(format!(
                    "Favro accepted the upload but returned an unexpected attachment: {response}"
                ));
            }
            let updated = find_card(&api, &collection, &card);
            let verified = updated
                .get("attachments")
                .and_then(|value| value.as_array())
                .is_some_and(|attachments| {
                    attachments.iter().any(|attachment| {
                        attachment.get("name").and_then(|value| value.as_str()) == Some(&filename)
                    })
                });
            if !verified {
                die(format!(
                    "Uploaded {filename}, but it did not appear when card {card} was re-read."
                ));
            }
            match &outgoing_url {
                None => {
                    println!(
                        "Attached {filename} ({} bytes, {mime_type}) to card {card}{}",
                        bytes.len(),
                        if replace {
                            " (no prior attachment shared this name; nothing was replaced)"
                        } else {
                            ""
                        }
                    );
                }
                Some(outgoing_url) => {
                    // The card never goes without the file: the old copy is only removed
                    // now, after the new upload is confirmed present.
                    let base = find_description_card(&api, &collection, &card);
                    let desc = base
                        .get("detailedDescription")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    api.write_description_removing(
                        &base,
                        desc,
                        &[outgoing_url.as_str()],
                        std::slice::from_ref(&filename),
                    );
                    let after = find_card(&api, &collection, &card);
                    let remaining: Vec<Option<&str>> = after
                        .get("attachments")
                        .and_then(Value::as_array)
                        .map(|attachments| {
                            attachments
                                .iter()
                                .filter(|attachment| {
                                    attachment.get("name").and_then(Value::as_str)
                                        == Some(filename.as_str())
                                })
                                .map(|attachment| attachment.get("fileURL").and_then(Value::as_str))
                                .collect()
                        })
                        .unwrap_or_default();
                    if remaining.len() != 1 || remaining[0] == Some(outgoing_url.as_str()) {
                        die(format!(
                            "Replace did not leave exactly one {filename:?} on card {card}; inspect the card before continuing."
                        ));
                    }
                    println!(
                        "Attached {filename} ({} bytes, {mime_type}) to card {card}, replacing the prior copy (unlinked from the card; Favro's storage may still retain the old file).",
                        bytes.len()
                    );
                }
            }
        }
        Cmd::Detach { card, name, url } => {
            let api = api_handle();
            let c = find_description_card(&api, &collection, &card);
            let same_name: Vec<&Value> = c
                .get("attachments")
                .and_then(Value::as_array)
                .map(|attachments| {
                    attachments
                        .iter()
                        .filter(|attachment| {
                            attachment.get("name").and_then(Value::as_str) == Some(name.as_str())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let outgoing: &Value = match (same_name.as_slice(), &url) {
                ([], _) => die(format!("Card {card} has no attachment named {name:?}.")),
                ([one], None) => one,
                (_, Some(target)) => same_name
                    .iter()
                    .find(|attachment| {
                        attachment.get("fileURL").and_then(Value::as_str) == Some(target.as_str())
                    })
                    .unwrap_or_else(|| {
                        die(format!(
                            "Card {card} has no attachment named {name:?} with fileURL {target:?}."
                        ))
                    }),
                (_, None) => die(format!(
                    "Card {card} has {} attachments named {name:?}; pass --url <fileURL> to pick one (see `favro get --card {card}`).",
                    same_name.len()
                )),
            };
            let outgoing_url = outgoing
                .get("fileURL")
                .and_then(Value::as_str)
                .unwrap_or_else(|| {
                    die(format!(
                        "Attachment {name:?} has no fileURL; cannot detach it safely."
                    ))
                })
                .to_string();
            let desc = c
                .get("detailedDescription")
                .and_then(Value::as_str)
                .unwrap_or("");
            api.write_description_removing(
                &c,
                desc,
                &[outgoing_url.as_str()],
                std::slice::from_ref(&name),
            );
            let after = find_card(&api, &collection, &card);
            let remaining = after
                .get("attachments")
                .and_then(Value::as_array)
                .is_some_and(|attachments| {
                    attachments.iter().any(|attachment| {
                        attachment.get("fileURL").and_then(Value::as_str)
                            == Some(outgoing_url.as_str())
                    })
                });
            if remaining {
                die(format!(
                    "Detach did not take effect: {name:?} is still on card {card} after the update."
                ));
            }
            println!(
                "Detached {name:?} from card {card} (unlinked from the card; Favro's storage may still retain the file)."
            );
        }
        // Both commands refuse to touch a human-authored comment. A comment is especially risky: there is no block marker to aim at and no copy anywhere else. The
        // 🤖 marker is the only signal of authorship the API gives us that we control, so
        // it is the gate. If a human comment genuinely must go, that is a Favro-UI action.
        Cmd::CommentEdit {
            card,
            comment,
            text,
            raw,
        } => {
            let api = api_handle();
            fetch_bot_comment(&api, &card, &comment, "edit");
            let text = unescape(&text);
            let body = if raw {
                text
            } else {
                format!("{} {text}", author_prefix())
            };
            api.request(
                "PUT",
                &format!("/comments/{comment}"),
                &[],
                Some(json!({"comment": body})),
                None,
            );
            println!("Rewrote comment {comment} on card {card}");
        }
        Cmd::CommentDelete { card, comment } => {
            let api = api_handle();
            fetch_bot_comment(&api, &card, &comment, "delete");
            api.request("DELETE", &format!("/comments/{comment}"), &[], None, None);
            println!("Deleted comment {comment} from card {card}");
        }
        Cmd::SetNotes { card, text } => {
            const START: &str = "<!-- 🤖 NOTES -->";
            const END: &str = "<!-- /🤖 NOTES -->";
            let api = api_handle();
            let c = find_description_card(&api, &collection, &card);
            let desc = checkboxes_to_markdown(
                c.get("detailedDescription")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            );
            let block = format!("{START}\n{}\n{END}", unescape(&text));
            let newdesc = place_managed_block(&desc, START, END, &block, false);
            api.write_description(&c, &newdesc);
            println!("Updated 🤖 notes on {card}");
        }
        Cmd::SetTodo {
            card,
            item,
            note,
            participant,
            blocked,
            clear,
        } => {
            const START: &str = "<!-- 👤 TODO -->";
            const END: &str = "<!-- /👤 TODO -->";
            if item.is_empty() && !clear {
                die("Nothing to do: pass at least one --item, or --clear to remove the block.");
            }
            if clear && !item.is_empty() {
                die("--clear takes no --item.");
            }
            let participant_name = participant.as_deref().or_else(|| {
                project
                    .as_ref()
                    .and_then(|config| config.project.default_human.as_deref())
            });
            if !clear
                && project
                    .as_ref()
                    .is_some_and(|config| !config.compatibility.allow_unassigned_handoffs)
                && participant_name.is_none()
            {
                die("A human action needs an assignee. Pass --participant or configure project.default_human.");
            }
            let participant_id = match (project.as_ref(), participant_name) {
                (Some(config), Some(name)) => {
                    Some(assignment_id(config, name).unwrap_or_else(|e| die(e)))
                }
                _ => None,
            };
            let api = api_handle();
            let c = find_description_card(&api, &collection, &card);
            let card_id = c
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let handoff_location = if clear {
                None
            } else {
                project.as_ref().map(|config| {
                    let target = if blocked {
                        &config.workflow.blocked
                    } else {
                        &config.workflow.waiting
                    };
                    let wid = c
                        .get("widgetCommonId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let column = api
                        .paginate("/columns", &[("widgetCommonId", wid.clone())])
                        .into_iter()
                        .find(|column| {
                            column.get("name").and_then(Value::as_str) == Some(target.as_str())
                        })
                        .and_then(|column| {
                            column
                                .get("columnId")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| {
                            die(format!(
                                "Card board has no configured handoff lane {target:?}."
                            ))
                        });
                    (wid, column)
                })
            };
            let desc = checkboxes_to_markdown(
                c.get("detailedDescription")
                    .and_then(|v| v.as_str())
                    .unwrap_or(""),
            );

            // Strip any existing block first, so the rebuilt one always lands back at the top.
            let rest = remove_managed_block(&desc, START, END);

            let newdesc = if clear {
                rest.trim_start_matches(['\n', '\r']).to_string()
            } else {
                let mut block = String::from(START);
                block.push_str("\n## 👤 Needs you\n");
                for i in &item {
                    block.push_str(&format!("- [ ] {}\n", unescape(i).trim()));
                }
                if let Some(n) = &note {
                    block.push_str(&format!("\n{}\n", unescape(n).trim()));
                }
                block.push_str(END);
                place_managed_block(&rest, START, END, &block, true)
            };

            api.write_description(&c, &newdesc);

            if let Some(config) = project.as_ref() {
                let mut handoff = serde_json::Map::new();
                if clear {
                    if let Some(human_id) = participant_id {
                        handoff.insert("removeAssignmentIds".into(), json!([human_id]));
                    }
                    if let Some((_, role)) =
                        config.role(cli.role.as_deref()).unwrap_or_else(|e| die(e))
                    {
                        if let Some(agent_id) = role
                            .auth
                            .as_ref()
                            .and_then(|a| config.auth.get(a))
                            .and_then(|a| a.user_id.clone())
                        {
                            handoff.insert("addAssignmentIds".into(), json!([agent_id]));
                        }
                    }
                } else {
                    if let Some(human_id) = participant_id {
                        handoff.insert("addAssignmentIds".into(), json!([human_id]));
                    }
                    let (wid, column) = handoff_location
                        .as_ref()
                        .expect("handoff location was validated before description mutation");
                    handoff.insert("widgetCommonId".into(), json!(wid));
                    handoff.insert("columnId".into(), json!(column));
                }
                if !handoff.is_empty() {
                    api.request(
                        "PUT",
                        &format!("/cards/{card_id}"),
                        &[],
                        Some(Value::Object(handoff)),
                        None,
                    );
                    api.verify_attachments(&c, &card_id, &[]);
                }
            }
            if clear {
                println!("Cleared the 👤 Needs-you checklist on {card}");
            } else {
                println!(
                    "Pinned a 👤 Needs-you checklist ({} item(s)) to the top of {card}",
                    item.len()
                );
            }
        }
        Cmd::SetDesc { card, text } => {
            let api = api_handle();
            let c = find_description_card(&api, &collection, &card);
            api.write_description(&c, &unescape(&text));
            println!("Updated description on {card}");
        }
        Cmd::SetResult { card, text } => {
            const START: &str = "<!-- RESULT -->";
            const END: &str = "<!-- /RESULT -->";
            let api = api_handle();
            let c = find_description_card(&api, &collection, &card);
            let desc = checkboxes_to_markdown(
                c.get("detailedDescription")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
            );
            let block = format!("{START}\nResult/evidence:\n{}\n{END}", unescape(&text));
            let updated = place_managed_block(&desc, START, END, &block, false);
            api.write_description(&c, &updated);
            println!("Updated durable result/evidence on {card}");
        }
        Cmd::Tag { card, add, remove } => {
            if add.is_empty() && remove.is_empty() {
                die("Nothing to do: pass --add and/or --remove.");
            }
            let api = api_handle();
            let c = find_card(&api, &collection, &card);
            let card_id = c
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let mut body = serde_json::Map::new();
            if !add.is_empty() {
                body.insert("addTags".into(), json!(add));
            }
            if !remove.is_empty() {
                body.insert("removeTags".into(), json!(remove));
            }
            api.request(
                "PUT",
                &format!("/cards/{card_id}"),
                &[],
                Some(Value::Object(body)),
                None,
            );
            println!("Tagged {card}  +{add:?} -{remove:?}");
        }
        Cmd::Depend { card, on, before } => {
            let api = api_handle();
            let a_id = find_card(&api, &collection, &card)
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let b_id = find_card(&api, &collection, &on)
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            // isBefore=true means the --on card must complete before --card (default).
            let is_before = !before;
            api.request(
                "POST",
                &format!("/cards/{a_id}/dependencies"),
                &[],
                Some(json!({"dependencies": [{"cardId": b_id, "isBefore": is_before}]})),
                None,
            );
            if before {
                println!("Dependency set: {card} must finish before {on}.");
            } else {
                println!("Dependency set: {card} depends on {on} ({on} must finish first).");
            }
        }
        Cmd::Deps { card, json } => {
            let api = api_handle();
            let card_id = find_card(&api, &collection, &card)
                .get("cardId")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let (body, _) = api.request(
                "GET",
                &format!("/cards/{card_id}/dependencies"),
                &[],
                None,
                None,
            );
            let deps = body
                .get("dependencies")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if json {
                println!("{}", serde_json::to_string_pretty(&deps).unwrap());
                return;
            }
            if deps.is_empty() {
                println!("No dependencies on {card}.");
            }
            for d in &deps {
                let ccid = d
                    .get("cardCommonId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                // Resolve the linked card to a readable "#seq name".
                let label = api
                    .paginate("/cards", &[("cardCommonId", ccid.to_string())])
                    .first()
                    .map(|c| {
                        format!(
                            "#{} {}",
                            c.get("sequentialId")
                                .map(|v| v.to_string())
                                .unwrap_or("?".into()),
                            c.get("name").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .unwrap_or_else(|| ccid.to_string());
                if d.get("isBefore").and_then(|v| v.as_bool()).unwrap_or(false) {
                    println!("depends on {label}  (must finish first)");
                } else {
                    println!("blocks {label}  (this card is a prerequisite)");
                }
            }
        }
        Cmd::Get { card } => {
            let api = api_handle();
            println!(
                "{}",
                serde_json::to_string_pretty(&find_card(&api, &collection, &card)).unwrap()
            );
        }
        Cmd::Comments { card, json } => {
            let api = api_handle();
            let comments = api.paginate("/comments", &[("cardCommonId", card.clone())]);
            if json {
                println!("{}", serde_json::to_string_pretty(&comments).unwrap());
            } else if comments.is_empty() {
                println!("No comments on {card}.");
            } else {
                // Resolve userId -> display name once so lines read human-friendly.
                let mut users = std::collections::HashMap::new();
                for u in api.paginate("/users", &[]) {
                    if let (Some(id), Some(name)) = (
                        u.get("userId").and_then(|v| v.as_str()),
                        u.get("name").and_then(|v| v.as_str()),
                    ) {
                        users.insert(id.to_string(), name.to_string());
                    }
                }
                for c in &comments {
                    let who = c
                        .get("userId")
                        .and_then(|v| v.as_str())
                        .and_then(|id| users.get(id).map(|s| s.as_str()))
                        .unwrap_or("?");
                    let when = c.get("created").and_then(|v| v.as_str()).unwrap_or("");
                    let text = c.get("comment").and_then(|v| v.as_str()).unwrap_or("");
                    println!("── {who}  {when}\n{text}\n");
                }
            }
        }
        Cmd::List { board, lane } => {
            let (wid, cols) = resolve_board(&collection, &board, false, &lanes, false);
            let mut params = vec![("widgetCommonId", wid)];
            if let Some(l) = &lane {
                let col = cols.get(l).and_then(|v| v.as_str()).unwrap_or_else(|| {
                    die(format!(
                        "Unknown lane {l:?}; configured lanes are {lanes:?}"
                    ))
                });
                params.push(("columnId", col.to_string()));
            }
            let names: std::collections::HashMap<String, String> = cols
                .iter()
                .map(|(k, v)| (v.as_str().unwrap_or("").to_string(), k.clone()))
                .collect();
            let api = api_handle();
            let mut hidden = 0usize;
            for c in api.paginate("/cards", &params) {
                // The cards endpoint returns archived cards too; hiding them is the
                // whole point of archiving, so filter here rather than at the API.
                if c.get("archived").and_then(|v| v.as_bool()).unwrap_or(false) {
                    hidden += 1;
                    continue;
                }
                let lane = c
                    .get("columnId")
                    .and_then(|v| v.as_str())
                    .and_then(|id| names.get(id))
                    .map(|s| s.as_str())
                    .unwrap_or("?");
                println!(
                    "[{:<8}] #{:<5} {}  [{}]",
                    lane,
                    c.get("sequentialId")
                        .map(|v| v.to_string())
                        .unwrap_or("?".into()),
                    c.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                    c.get("cardCommonId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                );
            }
            if hidden > 0 {
                println!("({hidden} archived card(s) hidden)");
            }
        }
        Cmd::QaQueue { board } | Cmd::ReviewQueue { board } => {
            let boards: Vec<String> = if board.is_empty() {
                all_board_names(&collection)
            } else {
                board
            };
            let api = api_handle();
            let mut total = 0;
            let mut skipped = 0;
            let mut archived = 0;
            for b in boards {
                let (wid, cols) = resolve_board(&collection, &b, false, &lanes, false);
                let Some(review) = cols.get(&review_lane).and_then(Value::as_str) else {
                    eprintln!(
                        "Warning: skipping board {b:?}; it has no configured review lane {review_lane:?}."
                    );
                    skipped += 1;
                    continue;
                };
                let review = review.to_string();
                for c in api.paginate("/cards", &[("widgetCommonId", wid), ("columnId", review)]) {
                    if c.get("archived").and_then(Value::as_bool) == Some(true) {
                        archived += 1;
                        continue;
                    }
                    total += 1;
                    println!(
                        "{:<14} #{:<5} {}  [{}]",
                        b,
                        c.get("sequentialId")
                            .map(|v| v.to_string())
                            .unwrap_or("?".into()),
                        c.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        c.get("cardCommonId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("?")
                    );
                }
            }
            if total == 0 {
                println!("{review_lane} queue is empty.");
            }
            if skipped > 0 || archived > 0 {
                eprintln!(
                    "Review queue summary: skipped {skipped} board(s) without {review_lane:?}; hidden {archived} archived card(s)."
                );
            }
        }
        Cmd::Overview => {
            let api = api_handle();
            let mut total = 0usize;
            for board in all_board_names(&collection) {
                let (wid, cols) = resolve_board(&collection, &board, false, &lanes, false);
                let names: std::collections::HashMap<String, String> = cols
                    .iter()
                    .map(|(name, id)| (id.as_str().unwrap_or("").to_string(), name.clone()))
                    .collect();
                for card in api.paginate("/cards", &[("widgetCommonId", wid)]) {
                    if card.get("archived").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    total += 1;
                    let lane = card
                        .get("columnId")
                        .and_then(Value::as_str)
                        .and_then(|id| names.get(id))
                        .map(String::as_str)
                        .unwrap_or("?");
                    println!(
                        "{:<18} [{:<12}] #{:<5} {}  [{}]",
                        board,
                        lane,
                        card.get("sequentialId")
                            .map(Value::to_string)
                            .unwrap_or_else(|| "?".into()),
                        card.get("name").and_then(Value::as_str).unwrap_or(""),
                        card.get("cardCommonId")
                            .and_then(Value::as_str)
                            .unwrap_or("?")
                    );
                }
            }
            if total == 0 {
                println!("No live cards in configured collection.");
            }
        }
        Cmd::Assign { card, add, remove } => {
            let config = project
                .as_ref()
                .unwrap_or_else(|| die("Assignments by name require project configuration."));
            if add.is_empty() && remove.is_empty() {
                die("Pass --add and/or --remove with a configured participant or role.");
            }
            let add_ids: Vec<String> = add
                .iter()
                .map(|name| assignment_id(config, name).unwrap_or_else(|e| die(e)))
                .collect();
            let remove_ids: Vec<String> = remove
                .iter()
                .map(|name| assignment_id(config, name).unwrap_or_else(|e| die(e)))
                .collect();
            let api = api_handle();
            let card_id = find_card(&api, &collection, &card)
                .get("cardId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let mut body = serde_json::Map::new();
            if !add_ids.is_empty() {
                body.insert("addAssignmentIds".into(), json!(add_ids));
            }
            if !remove_ids.is_empty() {
                body.insert("removeAssignmentIds".into(), json!(remove_ids));
            }
            api.request(
                "PUT",
                &format!("/cards/{card_id}"),
                &[],
                Some(Value::Object(body)),
                None,
            );
            println!("Updated assignments on {card}: +{add:?} -{remove:?}");
        }
        Cmd::Priority {
            card,
            value,
            reason,
        } => {
            let config = project
                .as_ref()
                .unwrap_or_else(|| die("Priority requires project configuration."));
            if value.eq_ignore_ascii_case("Critical") && reason.as_deref().is_none_or(str::is_empty)
            {
                die("Critical priority requires --reason.");
            }
            let spec = config
                .fields
                .priority
                .as_ref()
                .unwrap_or_else(|| die("Configure [fields.priority] first."));
            let api = api_handle();
            let field = find_shared_field(&api, spec);
            let field_id = shared_field_id(&field);
            let item_id = field_item_id(&field, &value);
            let card_id = find_card(&api, &collection, &card)
                .get("cardId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            api.request(
                "PUT",
                &format!("/cards/{card_id}"),
                &[],
                Some(json!({
                    "customFields": [{"customFieldId": field_id, "value": [item_id]}]
                })),
                None,
            );
            if let Some(reason) = reason {
                api.request("POST", "/comments", &[], Some(json!({
                    "cardCommonId": card,
                    "comment": format!("{} Priority set to {value}: {}", author_prefix(), unescape(&reason))
                })), None);
            }
            println!("Set priority on {card} to {value}");
        }
        Cmd::Complexity { card, value } => {
            if !(1..=5).contains(&value) {
                die("Complexity must be 1 (low) through 5 (high).");
            }
            let config = project
                .as_ref()
                .unwrap_or_else(|| die("Complexity requires project configuration."));
            let spec = config
                .fields
                .complexity
                .as_ref()
                .unwrap_or_else(|| die("Configure [fields.complexity] first."));
            let api = api_handle();
            let field = find_shared_field(&api, spec);
            if field
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| normalized_field_type(kind) != "rating")
            {
                die("Configured Complexity field must be a Favro Rating field.");
            }
            let field_id = shared_field_id(&field);
            let card_id = find_card(&api, &collection, &card)
                .get("cardId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            api.request(
                "PUT",
                &format!("/cards/{card_id}"),
                &[],
                Some(json!({
                    "customFields": [{"customFieldId": field_id, "total": value}]
                })),
                None,
            );
            println!("Set complexity on {card} to {value}/5");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        checkboxes_to_markdown, collection_cache, infer_mime_type, migrate_legacy_collection_cache,
        percent_encode_query_component, place_managed_block, select_card_instance, truncate_utf8,
        unescape, Cli,
    };
    use clap::Parser;
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn query_components_are_percent_encoded() {
        assert_eq!(
            percent_encode_query_component("review #1+.md"),
            "review%20%231%2B.md"
        );
        assert_eq!(percent_encode_query_component("blå.md"), "bl%C3%A5.md");
    }

    #[test]
    fn utf8_truncation_stops_at_a_character_boundary() {
        let text = format!("{}ætail", "a".repeat(199));
        assert_eq!(truncate_utf8(&text, 200), "a".repeat(199));
        assert_eq!(truncate_utf8("blå", 99), "blå");
    }

    #[test]
    fn user_text_unescapes_newlines_and_tabs() {
        assert_eq!(unescape(r"one\ntwo\tthree"), "one\ntwo\tthree");
    }

    #[test]
    fn favro_checkbox_and_blank_line_glyphs_round_trip() {
        assert_eq!(
            checkboxes_to_markdown("Heading\n↵Body\n  ☐ Q1\n☑ Q2\n☒ Q3"),
            "Heading\n\nBody\n  - [ ] Q1\n- [x] Q2\n- [x] Q3"
        );
    }

    #[test]
    fn managed_block_replaces_existing_content() {
        assert_eq!(
            place_managed_block(
                "before\n<!-- X -->old<!-- /X -->\nafter",
                "<!-- X -->",
                "<!-- /X -->",
                "<!-- X -->new<!-- /X -->",
                false
            ),
            "before\n<!-- X -->new<!-- /X -->\nafter"
        );
    }

    #[test]
    fn managed_block_cleans_orphan_end_marker() {
        let updated = place_managed_block(
            "body\n<!-- /X -->",
            "<!-- X -->",
            "<!-- /X -->",
            "<!-- X -->new<!-- /X -->",
            false,
        );
        assert_eq!(updated.matches("<!-- /X -->").count(), 1);
        assert_eq!(updated, "body\n\n<!-- X -->new<!-- /X -->");
    }

    #[test]
    fn managed_block_discards_stale_content_after_orphan_start() {
        let updated = place_managed_block(
            "body\n<!-- X -->stale automation text",
            "<!-- X -->",
            "<!-- /X -->",
            "<!-- X -->new<!-- /X -->",
            false,
        );
        assert_eq!(updated, "body\n\n<!-- X -->new<!-- /X -->");
        assert!(!updated.contains("stale"));
    }

    #[test]
    fn card_selection_prefers_active_instance_in_selected_collection() {
        let widgets = HashSet::from(["selected".to_string()]);
        let selected = select_card_instance(
            vec![
                json!({"cardId": "other", "widgetCommonId": "outside", "archived": false}),
                json!({"cardId": "old", "widgetCommonId": "selected", "archived": true}),
                json!({"cardId": "live", "widgetCommonId": "selected", "archived": false}),
            ],
            &widgets,
            "common",
            "Product",
        )
        .unwrap();
        assert_eq!(selected["cardId"], "live");
    }

    #[test]
    fn duplicate_active_card_instances_are_rejected() {
        let widgets = HashSet::from(["one".to_string(), "two".to_string()]);
        let result = select_card_instance(
            vec![
                json!({"cardId": "a", "widgetCommonId": "one", "archived": false}),
                json!({"cardId": "b", "widgetCommonId": "two", "archived": false}),
            ],
            &widgets,
            "common",
            "Product",
        );
        assert!(result.unwrap_err().contains("multiple active instances"));
    }

    #[test]
    fn mime_types_are_inferred_case_insensitively() {
        assert_eq!(infer_mime_type("report.MD"), "text/markdown");
        assert_eq!(infer_mime_type("diagram.svg"), "image/svg+xml");
        assert_eq!(
            infer_mime_type("payload.unknown"),
            "application/octet-stream"
        );
    }

    #[test]
    fn archive_instance_requires_undo() {
        assert!(Cli::try_parse_from([
            "favro",
            "archive",
            "--card",
            "common",
            "--undo",
            "--instance",
            "per-board",
        ])
        .is_ok());
        assert!(Cli::try_parse_from([
            "favro",
            "archive",
            "--card",
            "common",
            "--instance",
            "per-board",
        ])
        .is_err());
    }

    #[test]
    fn collection_argument_is_global() {
        let before =
            Cli::try_parse_from(["favro", "--collection", "Product A", "list-collections"])
                .unwrap();
        assert_eq!(before.collection.as_deref(), Some("Product A"));

        let after = Cli::try_parse_from(["favro", "list-collections", "--collection", "Product B"])
            .unwrap();
        assert_eq!(after.collection.as_deref(), Some("Product B"));
    }

    #[test]
    fn legacy_cache_import_is_explicit_and_pure() {
        let legacy = json!({
            "collectionId": "legacy-collection",
            "boards": {"agent": {"widgetCommonId": "legacy-board"}}
        });
        let mut state = json!({
            "organizations": {"org": {"collections": {"Configured Product": {
                "roleEmojis": {"product": "🧩"}
            }}}}
        });
        assert!(collection_cache(&state, "org", "Configured Product")
            .unwrap()
            .get("collectionId")
            .is_none());
        migrate_legacy_collection_cache(&mut state, &legacy, "org", "Configured Product");
        let migrated = collection_cache(&state, "org", "Configured Product").unwrap();
        assert_eq!(migrated["collectionId"], "legacy-collection");
        assert_eq!(
            migrated["boards"]["agent"]["widgetCommonId"],
            "legacy-board"
        );
        assert_eq!(migrated["roleEmojis"]["product"], "🧩");
    }
}
