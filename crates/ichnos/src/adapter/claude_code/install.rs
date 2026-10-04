//! Adds the `ichnos hook` entries from `hooks.json` to a Claude Code settings file, or removes them,
//! together with the `ichnos` skill that lets agents query the logs and the `ichnos` plugin that
//! asks the model for its own account of each turn when a debug profile is set.
//!
//! Only entries that run `ichnos hook` are touched; every other setting and hook is kept, in
//! its original order. Installing twice leaves one copy, so `install` also updates an older one.
//! The skill goes next to the settings file, in `skills/ichnos/SKILL.md`; the plugin in
//! `ichnos-plugin/`, which the settings' `env.CLAUDE_CODE_PLUGIN_DIRS` names so Claude Code loads
//! it. Claude Code reads that variable from the user settings file only.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use thiserror::Error;

/// The hook block users would otherwise copy by hand; the source of truth for what is installed.
const HOOKS_JSON: &str = include_str!("../../../../../hooks.json");

/// The skill that teaches agents to query the session databases; compiled in, so it always
/// matches the schema this binary writes.
const SKILL_MD: &str = include_str!("../../../../../skill/SKILL.md");

/// Folder of the skill, inside the `skills` folder next to the settings file.
const SKILL_DIR: &str = "ichnos";

/// The Claude Code plugin's files, by their path inside its folder; compiled in like the skill.
const PLUGIN_FILES: [(&str, &str); 6] = [
    (
        ".claude-plugin/plugin.json",
        include_str!("../../../../../plugin/.claude-plugin/plugin.json"),
    ),
    (
        "hooks/hooks.json",
        include_str!("../../../../../plugin/hooks/hooks.json"),
    ),
    (
        "hooks/register.ts",
        include_str!("../../../../../plugin/hooks/register.ts"),
    ),
    (
        "hooks/account.ts",
        include_str!("../../../../../plugin/hooks/account.ts"),
    ),
    (
        "hooks/validate.ts",
        include_str!("../../../../../plugin/hooks/validate.ts"),
    ),
    (
        "hooks/profiles.ts",
        include_str!("../../../../../plugin/hooks/profiles.ts"),
    ),
];

/// Folder of the plugin, next to the settings file.
const PLUGIN_DIR: &str = "ichnos-plugin";

/// The settings `env` variable listing the plugin folders Claude Code loads, separated like
/// `PATH`.
const PLUGIN_DIRS_VAR: &str = "CLAUDE_CODE_PLUGIN_DIRS";

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
    let plugin = plugin_path(path);
    add_hooks(&mut settings).map_err(|what| shape_error(path, what))?;
    add_plugin_dir(&mut settings, &plugin).map_err(|what| shape_error(path, what))?;
    let hooks = write_if_changed(path, &before, &settings)?;
    Ok(hooks
        .or(write_skill(&skill_path(path))?)
        .or(write_plugin(&plugin)?))
}

/// Remove the ichnos hooks from the settings file at `path`, if it exists, and the skill.
///
/// # Errors
///
/// Returns an [`InstallError`] when the settings or the skill can't be read, parsed or written.
pub fn uninstall(path: &Path) -> Result<Change, InstallError> {
    let plugin = plugin_path(path);
    let files = remove_skill(&skill_path(path))?.or(remove_plugin(&plugin)?);
    let Some(mut settings) = read_settings(path)? else {
        return Ok(files);
    };
    let before = settings.clone();
    remove_hooks(&mut settings).map_err(|what| shape_error(path, what))?;
    remove_plugin_dir(&mut settings, &plugin).map_err(|what| shape_error(path, what))?;
    Ok(write_if_changed(path, &before, &settings)?.or(files))
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

/// Where the plugin goes for the settings file at `settings`: `<its folder>/ichnos-plugin`, made
/// absolute, since Claude Code takes the folder from an environment variable.
pub fn plugin_path(settings: &Path) -> PathBuf {
    let dir = settings
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(PLUGIN_DIR);
    std::path::absolute(&dir).unwrap_or(dir)
}

/// Write [`PLUGIN_FILES`] into the folder `dir`, leaving files that are already up to date, and
/// delete the files in its `hooks` folder that an older version left and this one doesn't ship.
fn write_plugin(dir: &Path) -> Result<Change, InstallError> {
    let mut change = Change::Unchanged;
    for (name, contents) in PLUGIN_FILES {
        let path = dir.join(name);
        if fs::read_to_string(&path).is_ok_and(|text| text == contents) {
            continue;
        }
        let write_error = |source| InstallError::Write {
            path: path.clone(),
            source,
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(write_error)?;
        }
        fs::write(&path, contents).map_err(write_error)?;
        change = Change::Updated;
    }
    Ok(change.or(remove_stale_hooks(&dir.join("hooks"))?))
}

/// Delete the files in the plugin's `hooks` folder that aren't in [`PLUGIN_FILES`]. Claude Code
/// loads only what `hooks.json` names, so a leftover is harmless, but it hides which version runs.
fn remove_stale_hooks(hooks: &Path) -> Result<Change, InstallError> {
    let read_error = |source| InstallError::Read {
        path: hooks.to_owned(),
        source,
    };
    let mut change = Change::Unchanged;
    for entry in fs::read_dir(hooks).map_err(read_error)? {
        let path = entry.map_err(read_error)?.path();
        let shipped = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                PLUGIN_FILES
                    .iter()
                    .any(|(shipped, _)| shipped.strip_prefix("hooks/") == Some(name))
            });
        if shipped || !path.is_file() {
            continue;
        }
        fs::remove_file(&path).map_err(|source| InstallError::Write {
            path: path.clone(),
            source,
        })?;
        change = Change::Updated;
    }
    Ok(change)
}

/// Delete the plugin folder `dir`, with what Claude Code wrote into it (its types), if it holds
/// the `ichnos` plugin. A folder holding anything else is left alone.
fn remove_plugin(dir: &Path) -> Result<Change, InstallError> {
    let manifest = dir.join(PLUGIN_FILES[0].0);
    let is_ours = fs::read_to_string(&manifest)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|manifest| manifest["name"] == "ichnos");
    if !is_ours {
        return Ok(Change::Unchanged);
    }
    fs::remove_dir_all(dir).map_err(|source| InstallError::Write {
        path: dir.to_owned(),
        source,
    })?;
    Ok(Change::Updated)
}

/// Add `dir` to the settings' `env.CLAUDE_CODE_PLUGIN_DIRS`, keeping the folders already there.
fn add_plugin_dir(settings: &mut Value, dir: &Path) -> Result<(), String> {
    let env = object_entry(settings, "env")?;
    let mut dirs = plugin_dirs(env)?;
    if !dirs.iter().any(|known| known == dir) {
        dirs.push(dir.to_owned());
    }
    let joined = std::env::join_paths(&dirs).map_err(|_| format!("`env.{PLUGIN_DIRS_VAR}`"))?;
    env.insert(
        PLUGIN_DIRS_VAR.to_owned(),
        Value::String(joined.to_string_lossy().into_owned()),
    );
    Ok(())
}

/// Remove `dir` from the settings' `env.CLAUDE_CODE_PLUGIN_DIRS`, then the variable and `env`
/// if that emptied them.
fn remove_plugin_dir(settings: &mut Value, dir: &Path) -> Result<(), String> {
    let Value::Object(root) = settings else {
        return Err("the settings".into());
    };
    let Some(Value::Object(env)) = root.get_mut("env") else {
        return Ok(());
    };
    let dirs = plugin_dirs(env)?;
    if !dirs.iter().any(|known| known == dir) {
        return Ok(());
    }
    let kept: Vec<_> = dirs.into_iter().filter(|known| known != dir).collect();
    if kept.is_empty() {
        env.shift_remove(PLUGIN_DIRS_VAR);
    } else {
        let joined = std::env::join_paths(&kept).map_err(|_| format!("`env.{PLUGIN_DIRS_VAR}`"))?;
        env.insert(
            PLUGIN_DIRS_VAR.to_owned(),
            Value::String(joined.to_string_lossy().into_owned()),
        );
    }
    if env.is_empty() {
        root.shift_remove("env");
    }
    Ok(())
}

/// The folders in `env.CLAUDE_CODE_PLUGIN_DIRS`, none when it is missing.
fn plugin_dirs(env: &Map<String, Value>) -> Result<Vec<PathBuf>, String> {
    match env.get(PLUGIN_DIRS_VAR) {
        None => Ok(Vec::new()),
        Some(Value::String(dirs)) => Ok(std::env::split_paths(dirs)
            .filter(|dir| !dir.as_os_str().is_empty())
            .collect()),
        Some(_) => Err(format!("`env.{PLUGIN_DIRS_VAR}` to be text, and")),
    }
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
        assert_eq!(keys, ["model", "hooks", "env"]);
    }

    #[test]
    fn install_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new/.claude/settings.local.json");

        assert_eq!(install(&path).unwrap(), Change::Updated);

        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let mut wanted: Value = serde_json::from_str(HOOKS_JSON).unwrap();
        let plugin = plugin_path(&path).to_string_lossy().into_owned();
        wanted["env"] = json!({ PLUGIN_DIRS_VAR: plugin });
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
    fn the_plugin_is_installed_next_to_the_settings_and_named_in_its_env() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let other = "/opt/other-plugin";
        fs::write(
            &path,
            json!({ "env": { PLUGIN_DIRS_VAR: other, "FOO": "1" } }).to_string(),
        )
        .unwrap();
        let plugin = plugin_path(&path);
        assert!(plugin.is_absolute());

        install(&path).unwrap();
        for (name, contents) in PLUGIN_FILES {
            assert_eq!(
                fs::read_to_string(plugin.join(name)).unwrap(),
                contents,
                "{name}"
            );
        }
        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let expected = std::env::join_paths([Path::new(other), &plugin]).unwrap();
        assert_eq!(
            settings["env"][PLUGIN_DIRS_VAR],
            json!(expected.to_str().unwrap())
        );
        assert_eq!(install(&path).unwrap(), Change::Unchanged, "named once");

        fs::create_dir_all(plugin.join(".claude-plugin/types")).unwrap();
        uninstall(&path).unwrap();
        assert!(
            !plugin.exists(),
            "removed with what Claude Code wrote into it"
        );
        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            settings["env"],
            json!({ PLUGIN_DIRS_VAR: other, "FOO": "1" })
        );
    }

    #[test]
    fn install_deletes_hook_files_an_older_version_left() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let plugin = plugin_path(&path);
        fs::create_dir_all(plugin.join("hooks")).unwrap();
        fs::create_dir_all(plugin.join(".claude-plugin/types")).unwrap();
        fs::write(plugin.join("hooks/register.tsx"), "old").unwrap();
        fs::write(plugin.join(".claude-plugin/types/tsconfig.json"), "{}").unwrap();

        install(&path).unwrap();
        assert!(!plugin.join("hooks/register.tsx").exists());
        assert!(plugin.join("hooks/register.ts").exists());
        assert!(
            plugin.join(".claude-plugin/types/tsconfig.json").exists(),
            "what Claude Code wrote is kept"
        );
        assert_eq!(install(&path).unwrap(), Change::Unchanged);
    }

    #[test]
    fn uninstall_leaves_a_plugin_folder_that_is_not_ichnos() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let plugin = plugin_path(&path);
        fs::create_dir_all(plugin.join(".claude-plugin")).unwrap();
        fs::write(
            plugin.join(".claude-plugin/plugin.json"),
            r#"{"name": "mine"}"#,
        )
        .unwrap();

        uninstall(&path).unwrap();
        assert!(plugin.join(".claude-plugin/plugin.json").exists());
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
