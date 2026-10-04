//! The compact form of the model's account: the full form of `docs/ichnos-debug.schema.json`
//! with one-letter keys and `"v": 2`, which is what a model is asked to write. [`expand`] turns
//! it into the full form (`"v": 1`), which is what Ichnos stores.
//!
//! The [`prompt`] that asks a model for it, the [`schema`] its answer must match and the
//! [`expand`]sion all come from one key table, [`keys`], so they can't disagree. The Claude Code
//! plugin carries a copy of the prompts and schemas, `plugin/hooks/profiles.ts`.

use std::fmt::Write as _;

use serde_json::{Map, Value, json};

use super::{Profile, Section};

/// The `v` of a compact block.
pub const VERSION: u64 = 2;

/// Longest text the full form allows: a phrase, and a name (a skill, tool or source).
const MAX_PHRASE: usize = 200;
const MAX_NAME: usize = 80;

/// Most entries the full form allows in a list.
const MAX_ITEMS: usize = 5;

/// One key of the compact form.
#[derive(Debug)]
pub struct Key {
    /// The key in the compact form.
    pub short: &'static str,
    /// The key in the full form.
    pub long: &'static str,
    /// What goes there, as the model is told.
    pub about: &'static str,
    /// What it holds.
    pub kind: Kind,
}

/// What a key holds.
#[derive(Debug)]
pub enum Kind {
    /// A sentence, cut to 200 characters when expanded.
    Phrase,
    /// A skill, tool or source name, cut to 80 characters when expanded.
    Name,
    /// `true` or `false`.
    Flag,
    /// One of these words.
    Choice(&'static [&'static str]),
    /// Sentences, at most 5 kept.
    Phrases,
    /// Objects with these keys, at most 5 kept.
    Records(&'static [Key]),
    /// One object with these keys.
    Fields(&'static [Key]),
}

const fn key(short: &'static str, long: &'static str, about: &'static str, kind: Kind) -> Key {
    Key {
        short,
        long,
        about,
        kind,
    }
}

const TASK: &[Key] = &[
    key(
        "i",
        "intent",
        "how you read the request, in one line",
        Kind::Phrase,
    ),
    key(
        "a",
        "scope_added",
        "work you did that wasn't asked for",
        Kind::Phrases,
    ),
    key(
        "d",
        "scope_dropped",
        "parts of the request you didn't do",
        Kind::Phrases,
    ),
];

const DECISIONS: &[Key] = &[
    key(
        "c",
        "choices",
        "choices you made",
        Kind::Records(&[
            key("c", "chose", "what you chose", Kind::Phrase),
            key("o", "over", "the options you rejected", Kind::Phrases),
            key("w", "why", "why", Kind::Phrase),
        ]),
    ),
    key(
        "a",
        "assumptions",
        "things you assumed instead of checking or asking",
        Kind::Records(&[
            key("a", "assumed", "the assumption", Kind::Phrase),
            key(
                "k",
                "could_have_checked",
                "whether you could have checked it",
                Kind::Flag,
            ),
        ]),
    ),
];

const CONTEXT: &[Key] = &[
    key(
        "r",
        "relied_on",
        "files, docs or instructions that shaped your answer",
        Kind::Phrases,
    ),
    key(
        "m",
        "from_memory",
        "claims based on your training, not on anything read this session",
        Kind::Phrases,
    ),
    key(
        "x",
        "missing_info",
        "things you needed and couldn't find",
        Kind::Phrases,
    ),
];

const TOOLS: &[Key] = &[
    key(
        "f",
        "friction",
        "tools that got in your way",
        Kind::Records(&[
            key("t", "tool", "the tool", Kind::Name),
            key(
                "p",
                "problem",
                "what went wrong or didn't fit",
                Kind::Phrase,
            ),
            key("w", "workaround", "what you did instead", Kind::Phrase),
        ]),
    ),
    key(
        "w",
        "wished_for",
        "tools or access you lacked",
        Kind::Phrases,
    ),
];

const SKILLS: &[Key] = &[
    key(
        "c",
        "considered",
        "skills you thought about this turn, loaded or not",
        Kind::Records(&[
            key("k", "skill", "the skill", Kind::Name),
            key("l", "loaded", "whether you loaded it", Kind::Flag),
            key("r", "reason", "why", Kind::Phrase),
        ]),
    ),
    key(
        "s",
        "skipped_steps",
        "steps of a loaded skill you skipped",
        Kind::Records(&[
            key("k", "skill", "the skill", Kind::Name),
            key("s", "step", "the step, as the skill words it", Kind::Phrase),
            key("r", "reason", "why you skipped it", Kind::Phrase),
        ]),
    ),
    key(
        "g",
        "missing_guidance",
        "situations a skill didn't cover, so you improvised",
        Kind::Records(&[
            key("k", "skill", "the skill", Kind::Name),
            key("s", "situation", "the situation", Kind::Phrase),
            key(
                "n",
                "needed",
                "the guidance that would have helped",
                Kind::Phrase,
            ),
        ]),
    ),
    key(
        "u",
        "unclear_instructions",
        "instructions you had to interpret",
        Kind::Records(&[
            key(
                "s",
                "source",
                "where they are: a skill, a file or the prompt",
                Kind::Name,
            ),
            key("t", "text", "the wording, quoted", Kind::Phrase),
            key("r", "read_as", "how you read it", Kind::Phrase),
        ]),
    ),
];

/// Where an instruction came from and what it says.
const SOURCE: &[Key] = &[
    key(
        "w",
        "where",
        "where: a skill, CLAUDE.md, AGENTS.md, a memory or the prompt",
        Kind::Name,
    ),
    key("s", "says", "what it says", Kind::Phrase),
];

const RULES: &[Key] = &[key(
    "c",
    "conflicts",
    "instructions that disagreed",
    Kind::Records(&[
        key("a", "a", "one instruction", Kind::Fields(SOURCE)),
        key("b", "b", "the other", Kind::Fields(SOURCE)),
        key(
            "f",
            "followed",
            "which you followed",
            Kind::Choice(&["a", "b", "neither"]),
        ),
        key("r", "reason", "why", Kind::Phrase),
    ]),
)];

const OUTCOME: &[Key] = &[
    key(
        "s",
        "state",
        "where the work stands",
        Kind::Choice(&["done", "partial", "blocked", "question"]),
    ),
    key("r", "remaining", "work left undone", Kind::Phrases),
    key(
        "u",
        "unverified_claims",
        "things you stated without checking them this session",
        Kind::Phrases,
    ),
    key(
        "k",
        "risks",
        "what might break because of your changes",
        Kind::Phrases,
    ),
];

/// A section's key in the compact form.
#[must_use]
pub fn short(section: Section) -> &'static str {
    match section {
        Section::Task => "t",
        Section::Decisions => "d",
        Section::Context => "c",
        Section::Tools => "l",
        Section::Skills => "s",
        Section::Rules => "r",
        Section::Outcome => "o",
    }
}

/// A section's keys in the compact form.
#[must_use]
pub fn keys(section: Section) -> &'static [Key] {
    match section {
        Section::Task => TASK,
        Section::Decisions => DECISIONS,
        Section::Context => CONTEXT,
        Section::Tools => TOOLS,
        Section::Skills => SKILLS,
        Section::Rules => RULES,
        Section::Outcome => OUTCOME,
    }
}

/// The JSON Schema of a compact block with `profile`'s sections, in the subset structured
/// outputs accept: every object closed (`additionalProperties: false`) with every key required,
/// and no length limits, which [`expand`] applies instead.
#[must_use]
pub fn schema(profile: Profile) -> Value {
    let mut properties = Map::new();
    properties.insert(
        "v".to_owned(),
        json!({ "type": "integer", "enum": [VERSION] }),
    );
    for &section in profile.sections() {
        properties.insert(short(section).to_owned(), object_schema(keys(section)));
    }
    let required: Vec<&str> = properties.keys().map(String::as_str).collect();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

fn object_schema(keys: &[Key]) -> Value {
    let properties: Map<String, Value> = keys
        .iter()
        .map(|key| (key.short.to_owned(), kind_schema(&key.kind)))
        .collect();
    let required: Vec<&str> = keys.iter().map(|key| key.short).collect();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

fn kind_schema(kind: &Kind) -> Value {
    match kind {
        Kind::Phrase | Kind::Name => json!({ "type": "string" }),
        Kind::Flag => json!({ "type": "boolean" }),
        Kind::Choice(words) => json!({ "type": "string", "enum": words }),
        Kind::Phrases => json!({ "type": "array", "items": { "type": "string" } }),
        Kind::Records(keys) => json!({ "type": "array", "items": object_schema(keys) }),
        Kind::Fields(keys) => object_schema(keys),
    }
}

/// The prompt that asks a model, right after a turn and with that turn in its context, for its
/// account of the turn in the compact form with `profile`'s sections.
#[must_use]
pub fn prompt(profile: Profile) -> String {
    let mut keys_text = format!("\"v\": always {VERSION}.\n");
    let mut example = Map::new();
    example.insert("v".to_owned(), json!(VERSION));
    for &section in profile.sections() {
        let name = short(section);
        let _ = writeln!(
            keys_text,
            "\"{name}\" ({}): {}",
            section.name(),
            section.question()
        );
        for key in keys(section) {
            let _ = writeln!(keys_text, "  \"{}\": {}", key.short, describe(key));
        }
        example.insert(name.to_owned(), quiet(keys(section)));
    }
    format!(
        "Ichnos, the recorder of this session, asks for your own account of the turn that just \
         ended, so the user can find where skills, instructions and tools fall short. Don't \
         continue the work and don't use tools: answer with one JSON object and nothing else, \
         no prose and no code fence.\n\n\
         The object has these keys:\n{keys_text}\n\
         A turn with nothing to report looks like this:\n{}\n\n\
         - Report only what happened in the turn that just ended. Use [] when there is nothing \
         to report; never invent entries.\n\
         - Write every key. Keep each text under {MAX_PHRASE} characters, and at most \
         {MAX_ITEMS} entries per list.",
        Value::Object(example),
    )
}

/// What the prompt says about `key`: what goes there, and the keys of its entries.
fn describe(key: &Key) -> String {
    match &key.kind {
        Kind::Flag => format!("{}, true or false", key.about),
        Kind::Choice(words) => {
            let words: Vec<String> = words.iter().map(|word| format!("\"{word}\"")).collect();
            format!("{}: one of {}", key.about, words.join(", "))
        }
        Kind::Records(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|field| format!("\"{}\" {}", field.short, describe(field)))
                .collect();
            format!("{}. Each: {}", key.about, fields.join("; "))
        }
        Kind::Fields(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|field| format!("\"{}\" {}", field.short, describe(field)))
                .collect();
            format!("{} ({})", key.about, fields.join(", "))
        }
        Kind::Phrase | Kind::Name | Kind::Phrases => key.about.to_owned(),
    }
}

/// A section with nothing to report: empty lists, the first choice, a placeholder phrase.
fn quiet(keys: &[Key]) -> Value {
    Value::Object(
        keys.iter()
            .map(|key| {
                let value = match &key.kind {
                    Kind::Phrase | Kind::Name => json!(format!("<{}>", key.about)),
                    Kind::Flag => json!(false),
                    Kind::Choice(words) => json!(words[0]),
                    Kind::Phrases | Kind::Records(_) => json!([]),
                    Kind::Fields(fields) => quiet(fields),
                };
                (key.short.to_owned(), value)
            })
            .collect(),
    )
}

/// The full form (`"v": 1`) of the compact block `compact`, with the full form's limits
/// applied: text cut to its length, lists to their first 5 entries.
///
/// # Errors
///
/// Returns where `compact` breaks the compact form: a missing or wrong `v`, a key no section
/// has, or a value of the wrong type. Keys a section has but `compact` leaves out are fine.
pub fn expand(compact: &Value) -> Result<Value, String> {
    let Value::Object(fields) = compact else {
        return Err("not an object".to_owned());
    };
    if fields.get("v").and_then(Value::as_u64) != Some(VERSION) {
        return Err(format!("`v` is not {VERSION}"));
    }
    let mut full = Map::new();
    full.insert("v".to_owned(), json!(1));
    for (name, value) in fields.iter().filter(|(name, _)| *name != "v") {
        let section = Section::ALL
            .into_iter()
            .find(|&section| short(section) == name)
            .ok_or_else(|| format!("no section `{name}`"))?;
        let path = section.name();
        full.insert(path.to_owned(), expand_object(value, keys(section), path)?);
    }
    Ok(Value::Object(full))
}

fn expand_object(value: &Value, keys: &[Key], path: &str) -> Result<Value, String> {
    let Value::Object(fields) = value else {
        return Err(format!("`{path}` is not an object"));
    };
    let mut full = Map::new();
    for (name, value) in fields {
        let key = keys
            .iter()
            .find(|key| key.short == name)
            .ok_or_else(|| format!("`{path}` has no key `{name}`"))?;
        let path = format!("{path}.{}", key.long);
        full.insert(key.long.to_owned(), expand_value(value, &key.kind, &path)?);
    }
    Ok(Value::Object(full))
}

fn expand_value(value: &Value, kind: &Kind, path: &str) -> Result<Value, String> {
    let wrong = || format!("`{path}` has the wrong type");
    match kind {
        Kind::Phrase => cut(value, MAX_PHRASE).ok_or_else(wrong),
        Kind::Name => cut(value, MAX_NAME).ok_or_else(wrong),
        Kind::Flag => value.is_boolean().then(|| value.clone()).ok_or_else(wrong),
        Kind::Choice(words) => match value.as_str() {
            Some(word) if words.contains(&word) => Ok(value.clone()),
            _ => Err(format!("`{path}` is none of {words:?}")),
        },
        Kind::Phrases => list(value, path, |item| cut(item, MAX_PHRASE).ok_or_else(wrong)),
        Kind::Records(keys) => list(value, path, |item| expand_object(item, keys, path)),
        Kind::Fields(keys) => expand_object(value, keys, path),
    }
}

/// The first [`MAX_ITEMS`] entries of the list `value`, each turned by `item`.
fn list(
    value: &Value,
    path: &str,
    item: impl Fn(&Value) -> Result<Value, String>,
) -> Result<Value, String> {
    let Value::Array(items) = value else {
        return Err(format!("`{path}` is not a list"));
    };
    items
        .iter()
        .take(MAX_ITEMS)
        .map(item)
        .collect::<Result<_, _>>()
        .map(Value::Array)
}

/// The text `value` cut to `max` characters, or `None` when it isn't text.
fn cut(value: &Value, max: usize) -> Option<Value> {
    value
        .as_str()
        .map(|text| Value::String(text.chars().take(max).collect()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const FULL_SCHEMA: &str = include_str!("../../../../docs/ichnos-debug.schema.json");

    /// A compact value for `kind` with one entry in every list, so every key is exercised.
    fn sample(kind: &Kind) -> Value {
        match kind {
            Kind::Phrase | Kind::Name => json!("x"),
            Kind::Flag => json!(true),
            Kind::Choice(words) => json!(words[0]),
            Kind::Phrases => json!(["x"]),
            Kind::Records(keys) => json!([sample_object(keys)]),
            Kind::Fields(keys) => sample_object(keys),
        }
    }

    fn sample_object(keys: &[Key]) -> Value {
        Value::Object(
            keys.iter()
                .map(|key| (key.short.to_owned(), sample(&key.kind)))
                .collect(),
        )
    }

    fn sample_block(profile: Profile) -> Value {
        let mut block = json!({ "v": VERSION });
        for &section in profile.sections() {
            block[short(section)] = sample_object(keys(section));
        }
        block
    }

    #[test]
    fn every_key_expands_to_the_full_form() {
        let full = jsonschema::validator_for(&serde_json::from_str(FULL_SCHEMA).unwrap()).unwrap();
        for profile in Profile::ALL {
            let compact = sample_block(profile);
            let compact_schema = jsonschema::validator_for(&schema(profile)).unwrap();
            assert!(compact_schema.is_valid(&compact), "{}", profile.name());

            let expanded = expand(&compact).unwrap();
            let errors: Vec<_> = full.iter_errors(&expanded).map(|e| e.to_string()).collect();
            assert!(errors.is_empty(), "{}: {errors:?}", profile.name());
            let sections: Vec<_> = expanded.as_object().unwrap().keys().cloned().collect();
            let mut expected = vec!["v".to_owned()];
            expected.extend(profile.sections().iter().map(|s| s.name().to_owned()));
            assert_eq!(sections, expected, "{}", profile.name());
        }
    }

    #[test]
    fn the_schema_is_in_the_subset_structured_outputs_accept() {
        fn check(schema: &Value) {
            if let Some(Value::Object(properties)) = schema.get("properties") {
                assert_eq!(schema["additionalProperties"], json!(false));
                let required: Vec<_> = schema["required"].as_array().unwrap().iter().collect();
                assert_eq!(required.len(), properties.len(), "every key required");
                properties.values().for_each(check);
            }
            if let Some(items) = schema.get("items") {
                check(items);
            }
            for limit in ["minLength", "maxLength", "minItems", "maxItems"] {
                assert!(schema.get(limit).is_none(), "{limit} isn't supported");
            }
        }
        for profile in Profile::ALL {
            check(&schema(profile));
        }
    }

    /// The plugin's copy of the prompts and schemas, as this module writes them.
    fn plugin_profiles() -> String {
        let mut ts = String::from(
            "// Written by `crates/ichnos-core/src/debug/compact.rs`'s tests; don't edit by hand.\n\
             // After changing the key table there: ICHNOS_BLESS=1 cargo test -p ichnos-core debug\n\n\
             import type { Schema } from './validate'\n\n\
             /** Per debug profile: the prompt that asks for the account, and the schema it must match. */\n\
             export const profiles: Readonly<Record<string, { prompt: string; schema: Schema }>> = {\n",
        );
        for profile in Profile::ALL {
            let prompt = serde_json::to_string(&prompt(profile)).unwrap();
            let schema = serde_json::to_string(&schema(profile)).unwrap();
            writeln!(
                ts,
                "  {}: {{\n    prompt: {prompt},\n    schema: {schema},\n  }},",
                profile.name()
            )
            .unwrap();
        }
        ts.push_str("}\n");
        ts
    }

    #[test]
    fn the_plugin_carries_the_current_prompts_and_schemas() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugin/hooks/profiles.ts"
        );
        let expected = plugin_profiles();
        if std::env::var_os("ICHNOS_BLESS").is_some() {
            std::fs::write(path, &expected).unwrap();
        }
        assert!(
            std::fs::read_to_string(path).is_ok_and(|ts| ts == expected),
            "{path} is out of date: run ICHNOS_BLESS=1 cargo test -p ichnos-core debug"
        );
    }

    #[test]
    fn the_prompt_explains_every_key_and_shows_a_valid_quiet_answer() {
        for profile in Profile::ALL {
            let text = prompt(profile);
            for &section in profile.sections() {
                assert!(text.contains(section.question()), "{}", profile.name());
                for key in keys(section) {
                    assert!(text.contains(key.about), "{}: {}", profile.name(), key.long);
                }
            }
            let example = text
                .lines()
                .find(|line| line.starts_with("{\"v\""))
                .unwrap();
            let example: Value = serde_json::from_str(example).unwrap();
            assert!(
                jsonschema::validator_for(&schema(profile))
                    .unwrap()
                    .is_valid(&example)
            );
            assert!(expand(&example).is_ok());
        }
    }

    #[test]
    fn expand_applies_the_full_form_limits() {
        let long = "y".repeat(500);
        let compact = json!({
            "v": 2,
            "s": { "c": [{ "k": long, "l": false, "r": long }] },
            "o": { "s": "done", "r": ["1", "2", "3", "4", "5", "6"], "u": [], "k": [] }
        });
        let full = expand(&compact).unwrap();
        let considered = &full["skills"]["considered"][0];
        assert_eq!(considered["skill"].as_str().unwrap().len(), MAX_NAME);
        assert_eq!(considered["reason"].as_str().unwrap().len(), MAX_PHRASE);
        assert_eq!(considered["loaded"], json!(false));
        assert_eq!(
            full["outcome"]["remaining"],
            json!(["1", "2", "3", "4", "5"])
        );
        assert_eq!(full["outcome"]["state"], json!("done"));
    }

    #[test]
    fn expand_names_what_breaks_the_compact_form() {
        let cases = [
            (json!({ "o": { "s": "done" } }), "`v` is not 2"),
            (json!({ "v": 2, "z": {} }), "no section `z`"),
            (
                json!({ "v": 2, "o": { "q": 1 } }),
                "`outcome` has no key `q`",
            ),
            (
                json!({ "v": 2, "o": { "s": "maybe" } }),
                "`outcome.state` is none of",
            ),
            (
                json!({ "v": 2, "s": { "c": [{ "l": "yes" }] } }),
                "`skills.considered.loaded` has the wrong type",
            ),
        ];
        for (compact, error) in cases {
            let got = expand(&compact).unwrap_err();
            assert!(got.starts_with(error), "{got}");
        }
    }
}
