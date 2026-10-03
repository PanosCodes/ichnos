//! Adds the `ichnos hook` entries from `hooks.json` to a Claude Code settings file, or removes them,
//! together with the `ichnos` skill that lets agents query the logs.
//!
//! Only entries that run `ichnos hook` are touched; every other setting and hook is kept, in
//! its original order. Installing twice leaves one copy, so `install` also updates an older one.
//! The skill goes next to the settings file, in `skills/ichnos/SKILL.md`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use thiserror::Error;

/// The hook block users would otherwise copy by hand; the source of truth for what is installed.
const HOOKS_JSON: &str = include_str!("../../../hooks.json");

/// The skill that teaches agents to query the session databases; compiled in, so it always
/// matches the schema this binary writes.
const SKILL_MD: &str = include_str!("../../../skill/SKILL.md");

/// Folder of the skill, inside the `skills` folder next to the settings file.
const SKILL_DIR: &str = "ichnos";

/// Why the settings file was left unchanged.
#[derive(Debug, Error)]
pub enum InstallError {
    #[error("could not read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("{path} is not valid JSON; fix it and try again")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("{path}: expected {what} to be a JSON object or array")]
    Shape { path: PathBuf, what: String },

    #[error("could not write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Whether the settings file was rewritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Updated,
    Unchanged,
}

impl Change {
    /// `Updated` when either change updated something.
    fn or(self, other: Self) -> Self {
        if self == Self::Updated { self } else { other }
    }
}

/// Add the ichnos hooks to the settings file at `path`, creating it if needed, and the skill.
///
/// # Errors
///
/// Returns an [`InstallError`] when the settings or the skill can't be read, parsed or written.
pub fn install(path: &Path) -> Result<Change, InstallError> {
    let mut settings = read_settings(path)?.unwrap_or_else(|| Value::Object(Map::new()));
    let before = settings.clone();
    add_hooks(&mut settings).map_err(|what| shape_error(path, what))?;
    let hooks = write_if_changed(path, &before, &settings)?;
    Ok(hooks.or(write_skill(&skill_path(path))?))
}

/// Remove the ichnos hooks from the settings file at `path`, if it exists, and the skill.
///
/// # Errors
///
/// Returns an [`InstallError`] when the settings or the skill can't be read, parsed or written.
pub fn uninstall(path: &Path) -> Result<Change, InstallError> {
    let skill = remove_skill(&skill_path(path))?;
    let Some(mut settings) = read_settings(path)? else {
        return Ok(skill);
    };
    let before = settings.clone();
    remove_hooks(&mut settings).map_err(|what| shape_error(path, what))?;
    Ok(write_if_changed(path, &before, &settings)?.or(skill))
}

/// Where the skill goes for the settings file at `settings`: `<its folder>/skills/ichnos/SKILL.md`.
pub fn skill_path(settings: &Path) -> PathBuf {
    settings
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("skills")
        .join(SKILL_DIR)
        .join("SKILL.md")
}

/// Write [`SKILL_MD`] to `path` unless it is already there.
fn write_skill(path: &Path) -> Result<Change, InstallError> {
    if fs::read_to_string(path).is_ok_and(|text| text == SKILL_MD) {
        return Ok(Change::Unchanged);
    }
    let write_error = |source| InstallError::Write {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(write_error)?;
    }
    fs::write(path, SKILL_MD).map_err(write_error)?;
    Ok(Change::Updated)
}

/// Delete the skill at `path` and its folder, if it exists. Other files in the folder are kept.
fn remove_skill(path: &Path) -> Result<Change, InstallError> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Change::Unchanged),
        Err(source) => {
            return Err(InstallError::Write {
                path: path.to_owned(),
                source,
            });
        }
    }
    if let Some(dir) = path.parent() {
        // Only succeeds when the folder is now empty.
        let _ = fs::remove_dir(dir);
    }
    Ok(Change::Updated)
}

/// Replace any ichnos hooks in `settings` with the ones from `hooks.json`.
fn add_hooks(settings: &mut Value) -> Result<(), String> {
    remove_hooks(settings)?;
    let wanted: Value = serde_json::from_str(HOOKS_JSON).map_err(|_| "hooks.json".to_owned())?;
    let Some(Value::Object(wanted)) = wanted.get("hooks").cloned() else {
        return Err("hooks.json `hooks`".into());
    };
    let hooks = object_entry(settings, "hooks")?;
    for (event, groups) in wanted {
        let Value::Array(groups) = groups else {
            return Err(format!("hooks.json `hooks.{event}`"));
        };
        let Value::Array(existing) = hooks
            .entry(event.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
        else {
            return Err(format!("`hooks.{event}`"));
        };
        existing.extend(groups);
    }
    Ok(())
}

/// Remove every hook that runs `ichnos hook`, then any group, event or `hooks` key it left empty.
fn remove_hooks(settings: &mut Value) -> Result<(), String> {
    let Value::Object(root) = settings else {
        return Err("the settings".into());
    };
    let Some(hooks) = root.get_mut("hooks") else {
        return Ok(());
    };
    let Value::Object(events) = hooks else {
        return Err("`hooks`".into());
    };
    let mut emptied_events = Vec::new();
    for (event, groups) in events.iter_mut() {
        let Value::Array(groups) = groups else {
            return Err(format!("`hooks.{event}`"));
        };
        let before = groups.len();
        let mut removed_any = false;
        groups.retain_mut(|group| {
            let Some(Value::Array(entries)) = group.get_mut("hooks") else {
                return true;
            };
            let count = entries.len();
            entries.retain(|entry| !is_ichnos_hook(entry));
            if entries.len() == count {
                return true;
            }
            removed_any = true;
            !entries.is_empty()
        });
        if removed_any && groups.is_empty() && before > 0 {
            emptied_events.push(event.clone());
        }
    }
    for event in &emptied_events {
        events.shift_remove(event);
    }
    if !emptied_events.is_empty() && events.is_empty() {
        root.shift_remove("hooks");
    }
    Ok(())
}

/// Whether a hook entry runs `ichnos hook`, by name or by path, with or without flags.
fn is_ichnos_hook(entry: &Value) -> bool {
    let Some(command) = entry.get("command").and_then(Value::as_str) else {
        return false;
    };
    let mut words = command.split_whitespace();
    let program = words.next().map(Path::new).and_then(Path::file_name);
    program.is_some_and(|name| name == "ichnos") && words.next() == Some("hook")
}

/// The object at `settings[key]`, inserting an empty one if it is missing.
fn object_entry<'a>(
    settings: &'a mut Value,
    key: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    let Value::Object(root) = settings else {
        return Err("the settings".into());
    };
    match root
        .entry(key.to_owned())
        .or_insert_with(|| Value::Object(Map::new()))
    {
        Value::Object(map) => Ok(map),
        _ => Err(format!("`{key}`")),
    }
}

/// The parsed settings file, or `None` when it doesn't exist.
fn read_settings(path: &Path) -> Result<Option<Value>, InstallError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(InstallError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    if text.trim().is_empty() {
        return Ok(Some(Value::Object(Map::new())));
    }
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| InstallError::Parse {
            path: path.to_owned(),
            source,
        })
}

/// Write `after` to `path` when it differs from `before`, replacing the file in one step.
///
/// A symlinked settings file (e.g. from a dotfiles repo) is written through, not replaced.
fn write_if_changed(path: &Path, before: &Value, after: &Value) -> Result<Change, InstallError> {
    if before == after {
        return Ok(Change::Unchanged);
    }
    let write_error = |source| InstallError::Write {
        path: path.to_owned(),
        source,
    };
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(write_error)?;
    }
    let mut text = serde_json::to_string_pretty(after).unwrap_or_default();
    text.push('\n');
    let temp = target.with_extension("json.ichnos-tmp");
    fs::write(&temp, text).map_err(write_error)?;
    fs::rename(&temp, &target).map_err(write_error)?;
    Ok(Change::Updated)
}

fn shape_error(path: &Path, what: String) -> InstallError {
    InstallError::Shape {
        path: path.to_owned(),
        what,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn ichnos_entry() -> Value {
        json!({ "type": "command", "command": "ichnos hook claude-code", "timeout": 10 })
    }

    #[test]
    fn install_keeps_other_settings_and_hooks_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude/settings.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mine = json!({ "type": "command", "command": "say done" });
        fs::write(
            &path,
            json!({ "model": "opus", "hooks": { "Stop": [{ "hooks": [mine] }] } }).to_string(),
        )
        .unwrap();

        assert_eq!(install(&path).unwrap(), Change::Updated);
        assert_eq!(install(&path).unwrap(), Change::Unchanged);

        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["model"], "opus");
        assert_eq!(
            settings["hooks"]["Stop"],
            json!([{ "hooks": [mine] }, { "hooks": [ichnos_entry()] }])
        );
        assert_eq!(settings["hooks"]["PreToolUse"][0]["matcher"], "*");
        let keys: Vec<_> = settings.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["model", "hooks"]);
    }

    #[test]
    fn install_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new/.claude/settings.local.json");

        assert_eq!(install(&path).unwrap(), Change::Updated);

        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let wanted: Value = serde_json::from_str(HOOKS_JSON).unwrap();
        assert_eq!(settings, wanted);
    }

    #[test]
    fn uninstall_removes_only_ichnos_and_what_it_emptied() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mine = json!({ "type": "command", "command": "say done" });
        let by_path = json!({ "type": "command", "command": "/usr/local/bin/ichnos hook" });
        fs::write(
            &path,
            json!({
                "model": "opus",
                "hooks": {
                    "Stop": [{ "hooks": [mine, ichnos_entry()] }],
                    "PreToolUse": [{ "matcher": "*", "hooks": [by_path] }],
                    "Notification": []
                }
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(uninstall(&path).unwrap(), Change::Updated);
        assert_eq!(uninstall(&path).unwrap(), Change::Unchanged);

        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            settings,
            json!({
                "model": "opus",
                "hooks": { "Stop": [{ "hooks": [mine] }], "Notification": [] }
            })
        );
    }

    #[test]
    fn install_then_uninstall_restores_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let original = json!({ "permissions": { "allow": ["Bash(ls)"] } });
        fs::write(&path, original.to_string()).unwrap();

        install(&path).unwrap();
        uninstall(&path).unwrap();

        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings, original);
    }

    #[test]
    fn uninstall_without_a_file_does_not_create_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");

        assert_eq!(uninstall(&path).unwrap(), Change::Unchanged);
        assert!(!path.exists());
    }

    #[test]
    fn the_skill_is_installed_next_to_the_settings_and_removed_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".claude/settings.json");
        let skill = dir.path().join(".claude/skills/ichnos/SKILL.md");
        assert_eq!(skill_path(&path), skill);

        install(&path).unwrap();
        assert_eq!(fs::read_to_string(&skill).unwrap(), SKILL_MD);

        fs::write(&skill, "old version").unwrap();
        assert_eq!(
            install(&path).unwrap(),
            Change::Updated,
            "a stale skill is replaced"
        );
        assert_eq!(fs::read_to_string(&skill).unwrap(), SKILL_MD);

        assert_eq!(uninstall(&path).unwrap(), Change::Updated);
        assert!(!skill.parent().unwrap().exists());
        assert!(
            dir.path().join(".claude/skills").exists(),
            "other skills' folder is kept"
        );
    }

    #[test]
    fn uninstall_removes_the_skill_even_without_a_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        install(&path).unwrap();
        fs::remove_file(&path).unwrap();

        assert_eq!(uninstall(&path).unwrap(), Change::Updated);
        assert!(!skill_path(&path).exists());
        assert!(!path.exists());
    }

    #[test]
    fn invalid_json_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ not json").unwrap();

        assert!(matches!(install(&path), Err(InstallError::Parse { .. })));
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn recognizes_ichnos_hook_commands() {
        let command = |c: &str| json!({ "command": c });
        assert!(is_ichnos_hook(&command("ichnos hook")));
        assert!(is_ichnos_hook(&command(
            "/Users/me/.cargo/bin/ichnos  hook"
        )));
        assert!(!is_ichnos_hook(&command("ichnos install")));
        assert!(is_ichnos_hook(&command("ichnos hook --dir /tmp/x")));
        assert!(is_ichnos_hook(&command("ichnos hook claude-code")));
        assert!(!is_ichnos_hook(&command("not-ichnos hook")));
    }
}
