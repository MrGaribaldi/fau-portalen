use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const GENERIC_LANES: [&str; 8] = [
    "Inbox",
    "Backlog",
    "Ready",
    "In Progress",
    "Waiting",
    "Blocked",
    "Review",
    "Done",
];

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub project: Project,
    #[serde(default)]
    pub workflow: Workflow,
    #[serde(default)]
    pub interviews: Interviews,
    #[serde(default)]
    pub credentials: CredentialSources,
    #[serde(default)]
    pub auth: HashMap<String, AuthProfile>,
    #[serde(default)]
    pub participants: HashMap<String, Participant>,
    #[serde(default)]
    pub roles: HashMap<String, Role>,
    #[serde(default)]
    pub fields: Fields,
    #[serde(default)]
    pub templates: Templates,
    #[serde(default)]
    pub completion: Completion,
    #[serde(default)]
    pub history: History,
    #[serde(default)]
    pub compatibility: Compatibility,
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub collection: String,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    pub default_human: Option<String>,
}

fn default_visibility() -> String {
    "users".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    #[serde(default = "generic_lanes")]
    pub lanes: Vec<String>,
    #[serde(default = "default_inbox")]
    pub inbox: String,
    #[serde(default = "default_backlog")]
    pub backlog: String,
    #[serde(default = "default_ready")]
    pub ready: String,
    #[serde(default = "default_doing")]
    pub doing: String,
    #[serde(default = "default_waiting")]
    pub waiting: String,
    #[serde(default = "default_blocked")]
    pub blocked: String,
    #[serde(default = "default_review")]
    pub review: String,
    #[serde(default = "default_done")]
    pub done: String,
}

impl Default for Workflow {
    fn default() -> Self {
        Self {
            lanes: generic_lanes(),
            inbox: default_inbox(),
            backlog: default_backlog(),
            ready: default_ready(),
            doing: default_doing(),
            waiting: default_waiting(),
            blocked: default_blocked(),
            review: default_review(),
            done: default_done(),
        }
    }
}

fn generic_lanes() -> Vec<String> {
    GENERIC_LANES.iter().map(|s| s.to_string()).collect()
}
fn default_inbox() -> String {
    "Inbox".into()
}
fn default_backlog() -> String {
    "Backlog".into()
}
fn default_ready() -> String {
    "Ready".into()
}
fn default_doing() -> String {
    "In Progress".into()
}
fn default_waiting() -> String {
    "Waiting".into()
}
fn default_blocked() -> String {
    "Blocked".into()
}
fn default_review() -> String {
    "Review".into()
}
fn default_done() -> String {
    "Done".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Interviews {
    #[serde(default = "default_granularity")]
    pub card_granularity: String,
}

impl Default for Interviews {
    fn default() -> Self {
        Self {
            card_granularity: default_granularity(),
        }
    }
}
fn default_granularity() -> String {
    "topic".into()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialSources {
    #[serde(default)]
    pub env_files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthProfile {
    pub email_env: String,
    pub token_env: String,
    pub organization_id_env: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Participant {
    pub user_id: String,
    pub email_env: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub emoji: String,
    pub label: Option<String>,
    pub board: String,
    pub auth: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    #[serde(default = "yes")]
    pub require_result: bool,
    #[serde(default = "yes")]
    pub require_review: bool,
}
impl Default for Completion {
    fn default() -> Self {
        Self {
            require_result: true,
            require_review: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub path: Option<String>,
    #[serde(default = "yes")]
    pub require_archive_reference: bool,
}
impl Default for History {
    fn default() -> Self {
        Self {
            path: None,
            require_archive_reference: true,
        }
    }
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    /// Explicit pre-0.2 single-project cache to import when the new cache is empty.
    pub legacy_cache_path: Option<String>,
    /// Permit checklist/lane handoffs without a native Favro assignee.
    #[serde(default)]
    pub allow_unassigned_handoffs: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Templates {
    #[serde(default = "default_work_template")]
    pub work: String,
    #[serde(default = "default_code_template")]
    pub code: String,
}

impl Default for Templates {
    fn default() -> Self {
        Self {
            work: default_work_template(),
            code: default_code_template(),
        }
    }
}
fn default_work_template() -> String {
    "Purpose/outcome:\nNext action:\nAcceptance:".into()
}
fn default_code_template() -> String {
    "Change:\nAcceptance:".into()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    pub priority: Option<SharedField>,
    pub complexity: Option<SharedField>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SharedField {
    pub name: String,
    pub id: Option<String>,
}

impl ProjectConfig {
    pub fn load(explicit: Option<&Path>) -> Result<Option<Self>, String> {
        let path = if let Some(path) = explicit {
            Some(path.to_path_buf())
        } else if let Ok(path) = std::env::var("FAVRO_PROJECT_CONFIG") {
            Some(PathBuf::from(path))
        } else {
            discover(std::env::current_dir().map_err(|e| e.to_string())?)
        };
        let Some(path) = path else { return Ok(None) };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("Cannot read Favro project config {}: {e}", path.display()))?;
        let raw: toml::Value = toml::from_str(&text)
            .map_err(|e| format!("Invalid Favro project config {}: {e}", path.display()))?;
        if raw
            .get("project")
            .and_then(|project| project.get("import_legacy_cache"))
            .is_some()
        {
            return Err(format!(
                "Invalid Favro project config {}: `project.import_legacy_cache` was removed. Configure `[compatibility] legacy_cache_path = \"/path/to/pre-0.2/favro-state.json\"` for cache import, and set `allow_unassigned_handoffs = true` there only if that legacy behavior is still required.",
                path.display()
            ));
        }
        // Parse the typed config from source text again so TOML preserves line/column
        // diagnostics for unknown keys and type errors.
        let mut config: Self = toml::from_str(&text)
            .map_err(|e| format!("Invalid Favro project config {}: {e}", path.display()))?;
        config.source = Some(path);
        config.validate()?;
        Ok(Some(config))
    }

    pub fn credential_files(&self) -> Vec<PathBuf> {
        let base = self
            .source
            .as_deref()
            .and_then(Path::parent)
            .unwrap_or_else(|| Path::new("."));
        self.credentials
            .env_files
            .iter()
            .map(|value| {
                let path = PathBuf::from(value);
                if path.is_absolute() {
                    path
                } else {
                    base.join(path)
                }
            })
            .collect()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.project.collection.trim().is_empty() {
            return Err("project.collection must not be empty".into());
        }
        if !["users", "organization", "public"].contains(&self.project.visibility.as_str()) {
            return Err("project.visibility must be users, organization, or public".into());
        }
        if self.workflow.lanes.is_empty() {
            return Err("workflow.lanes must not be empty".into());
        }
        let mut lanes = HashSet::new();
        for lane in &self.workflow.lanes {
            if lane.trim().is_empty() || !lanes.insert(lane) {
                return Err(format!(
                    "workflow lanes must be non-empty and unique: {lane:?}"
                ));
            }
        }
        for (purpose, lane) in [
            ("inbox", &self.workflow.inbox),
            ("backlog", &self.workflow.backlog),
            ("ready", &self.workflow.ready),
            ("doing", &self.workflow.doing),
            ("waiting", &self.workflow.waiting),
            ("blocked", &self.workflow.blocked),
            ("review", &self.workflow.review),
            ("done", &self.workflow.done),
        ] {
            if !lanes.contains(lane) {
                return Err(format!(
                    "workflow.{purpose}={lane:?} is not present in workflow.lanes"
                ));
            }
        }
        if !["topic", "question"].contains(&self.interviews.card_granularity.as_str()) {
            return Err("interviews.card_granularity must be topic or question".into());
        }
        let mut emojis = HashSet::new();
        for (name, role) in &self.roles {
            if role.emoji.trim().is_empty() || role.board.trim().is_empty() {
                return Err(format!("role {name:?} needs a non-empty emoji and board"));
            }
            if !emojis.insert(role.emoji.trim()) {
                return Err(format!(
                    "role {name:?} reuses emoji {:?}; role emojis must be unique within a project",
                    role.emoji
                ));
            }
            if let Some(auth) = &role.auth {
                if !self.auth.contains_key(auth) {
                    return Err(format!(
                        "role {name:?} references missing auth profile {auth:?}"
                    ));
                }
            }
        }
        for (name, profile) in &self.auth {
            for (label, env_name) in [
                ("email_env", profile.email_env.as_str()),
                ("token_env", profile.token_env.as_str()),
            ] {
                if env_name.trim().is_empty() {
                    return Err(format!("auth profile {name:?} has empty {label}"));
                }
            }
        }
        if let Some(human) = &self.project.default_human {
            if !self.participants.contains_key(human) {
                return Err(format!(
                    "project.default_human references missing participant {human:?}"
                ));
            }
        }
        Ok(())
    }

    pub fn role(&self, name: Option<&str>) -> Result<Option<(&str, &Role)>, String> {
        match name {
            Some(name) => self
                .roles
                .get_key_value(name)
                .map(|(k, v)| Some((k.as_str(), v)))
                .ok_or_else(|| format!("Unknown role {name:?}; configure it in [roles.{name}]")),
            None if self.roles.len() == 1 => {
                let (k, v) = self.roles.iter().next().unwrap();
                Ok(Some((k.as_str(), v)))
            }
            None => Ok(None),
        }
    }

    pub fn apply_auth(&self, role_name: Option<&str>) -> Result<(), String> {
        let Some((_, role)) = self.role(role_name)? else {
            return Ok(());
        };
        let Some(profile_name) = &role.auth else {
            return Ok(());
        };
        let profile = &self.auth[profile_name];
        copy_env(&profile.email_env, "FAVRO_EMAIL")?;
        copy_env(&profile.token_env, "FAVRO_TOKEN")?;
        if let Some(source) = &profile.organization_id_env {
            copy_env(source, "FAVRO_ORG_ID")?;
        }
        Ok(())
    }

    pub fn author_prefix(&self, role_name: Option<&str>) -> Result<Option<String>, String> {
        Ok(self.role(role_name)?.map(|(name, role)| {
            format!(
                "{} {}:",
                role.emoji.trim(),
                role.label.as_deref().unwrap_or(name)
            )
        }))
    }
}

fn copy_env(source: &str, target: &str) -> Result<(), String> {
    if source.trim().is_empty() {
        return Err(format!("credential environment name for {target} is empty"));
    }
    let value = std::env::var(source)
        .map_err(|_| format!("Credential environment variable {source} is not set"))?;
    std::env::set_var(target, value);
    Ok(())
}

fn discover(mut dir: PathBuf) -> Option<PathBuf> {
    loop {
        let candidate = dir.join(".favro/project.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn starter(collection: &str, visibility: &str, granularity: &str) -> String {
    format!(
        r#"[project]
collection = {collection:?}
visibility = {visibility:?}

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
card_granularity = {granularity:?}

[templates]
work = "Purpose/outcome:\nNext action:\nAcceptance:"
code = "Change:\nAcceptance:"

[completion]
require_result = true
require_review = true

[history]
require_archive_reference = true
# path = "docs/project-history.md"

[compatibility]
# legacy_cache_path = "/absolute/path/to/pre-0.2/favro-state.json"
allow_unassigned_handoffs = false

# Secrets never belong here. Add auth profiles whose values name environment variables:
# [auth.primary]
# email_env = "FAVRO_PRIMARY_EMAIL"
# token_env = "FAVRO_PRIMARY_TOKEN"
# organization_id_env = "FAVRO_PRIMARY_ORG_ID"
# user_id = "Favro user ID for assignment"

# Add stable workstream roles with unique, project-immutable emoji prefixes:
# [roles.product]
# emoji = "🧩"
# label = "Product"
# board = "Product"
# auth = "primary"

# [participants.owner]
# user_id = "Favro user ID"
# email_env = "FAVRO_OWNER_EMAIL"
# Then set project.default_human = "owner" above.

# Existing shared fields are discovered by name; Favro's published API cannot create them:
# [fields.priority]
# name = "Priority"
# [fields.complexity]
# name = "Complexity"
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(extra: &str) -> ProjectConfig {
        toml::from_str(&format!(
            r#"
[project]
collection = "Example"
visibility = "users"

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
{extra}
"#
        ))
        .unwrap()
    }

    #[test]
    fn generic_project_is_valid() {
        parse("").validate().unwrap();
    }

    #[test]
    fn duplicate_role_emojis_are_rejected() {
        let config = parse(
            r#"
[roles.product]
emoji = "🧩"
board = "Product"
[roles.market]
emoji = "🧩"
board = "Market"
"#,
        );
        assert!(config.validate().unwrap_err().contains("reuses emoji"));
    }

    #[test]
    fn semantic_lane_must_exist() {
        let mut config = parse("");
        config.workflow.review = "Approval".into();
        assert!(config.validate().unwrap_err().contains("workflow.review"));
    }

    fn load_text(text: &str) -> Result<Option<ProjectConfig>, String> {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "favro-config-test-{}-{unique}.toml",
            std::process::id()
        ));
        std::fs::write(&path, text).unwrap();
        let result = ProjectConfig::load(Some(&path));
        std::fs::remove_file(path).unwrap();
        result
    }

    #[test]
    fn unknown_keys_are_rejected_through_project_load() {
        let legacy = r#"
[project]
collection = "Example"
import_legacy_cache = true
"#;
        let error = load_text(legacy).unwrap_err();
        assert!(error.contains("import_legacy_cache"));
        assert!(error.contains("[compatibility]"));

        let typo = r#"
[project]
collection = "Example"
[completion]
require_reveiw = false
"#;
        let error = load_text(typo).unwrap_err();
        assert!(error.contains("require_reveiw"));
        assert!(error.contains("line 5, column 1"));
    }

    #[test]
    fn starter_contains_no_secret_values() {
        let text = starter("Example", "users", "topic");
        let parsed: ProjectConfig = toml::from_str(&text).unwrap();
        parsed.validate().unwrap();
        assert!(!text.contains("token ="));
        assert!(text.contains("token_env"));
    }
}
