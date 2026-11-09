//! The configuration as `section.key` pairs, and the edits that carry a change into the user's
//! file through `toml_edit`, so everything the change does not touch keeps its exact text.

use std::collections::BTreeMap;

use toml_edit::{DocumentMut, Item};

use crate::config::{Config, ConfigError};

/// Every `section.key` of `config` with its TOML value. Optional keys that are unset are absent.
pub(crate) fn flatten(config: &Config) -> BTreeMap<String, toml::Value> {
    let mut keys = BTreeMap::new();
    let Ok(toml::Value::Table(sections)) = toml::Value::try_from(config) else {
        return keys;
    };
    for (section, table) in sections {
        if let toml::Value::Table(table) = table {
            for (key, value) in table {
                keys.insert(format!("{section}.{key}"), value);
            }
        }
    }
    keys
}

/// The configuration that `keys` (as [`flatten`] gives them) describe, validated.
///
/// # Errors
/// [`ConfigError::Parse`] for a value of the wrong type or an unknown key, otherwise the first
/// [`Config::validate`] error.
pub(crate) fn unflatten(keys: &BTreeMap<String, toml::Value>) -> Result<Config, ConfigError> {
    let mut sections = toml::Table::new();
    for (path, value) in keys {
        let (section, key) = path.split_once('.').unwrap_or(("", path));
        let entry = sections
            .entry(section.to_owned())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if let toml::Value::Table(table) = entry {
            table.insert(key.to_owned(), value.clone());
        }
    }
    let config: Config = toml::Value::Table(sections).try_into()?;
    config.validate()?;
    Ok(config)
}

/// Writes into `document` every key whose value differs between `before` and `after`: a value
/// equal to the default (or unset) is removed, any other is set in place, keeping the comment
/// after it. Keys that did not change are left exactly as written, even when they hold the
/// default. Returns whether the document changed.
pub(crate) fn patch(document: &mut DocumentMut, before: &Config, after: &Config) -> bool {
    let defaults = flatten(&Config::default());
    let old = flatten(before);
    let new = flatten(after);
    let mut changed = false;
    for path in old.keys().chain(new.keys()) {
        if old.get(path) == new.get(path) {
            continue;
        }
        let Some((section, key)) = path.split_once('.') else {
            continue;
        };
        changed |= match new.get(path) {
            Some(value) if defaults.get(path) != Some(value) => set(document, section, key, value),
            _ => remove(document, section, key),
        };
    }
    changed
}

/// Removes every key the configuration owns from `document`, leaving comments, unknown keys
/// and tables that still hold anything. Returns whether the document changed.
pub(crate) fn clear(document: &mut DocumentMut) -> bool {
    let mut changed = false;
    for path in flatten(&every_key_set()).keys() {
        if let Some((section, key)) = path.split_once('.') {
            changed |= remove(document, section, key);
        }
    }
    changed
}

/// A configuration with every optional key set, so [`flatten`] lists all of them.
pub(crate) fn every_key_set() -> Config {
    let mut config = Config::default();
    let colour = Some("#000000".to_owned());
    config.overlay.background.clone_from(&colour);
    config.overlay.foreground.clone_from(&colour);
    config.overlay.typed_foreground.clone_from(&colour);
    config.overlay.border = colour;
    config
}

/// Sets `section.key` to `value`, creating the table when needed and keeping the decoration
/// (spacing and trailing comment) of a value already there.
fn set(document: &mut DocumentMut, section: &str, key: &str, value: &toml::Value) -> bool {
    let Some(mut new) = edit_value(value) else {
        return false;
    };
    if !document.contains_key(section) {
        let mut table = toml_edit::Table::new();
        table.set_implicit(false);
        if document.is_empty() {
            // A file of comments only: they stay above the first table, not below it.
            let header = raw(Some(document.trailing()));
            table.decor_mut().set_prefix(header);
            document.set_trailing("");
        }
        document.insert(section, Item::Table(table));
    }
    let Some(table) = document.get_mut(section).and_then(Item::as_table_like_mut) else {
        return false;
    };
    match table.get_mut(key) {
        Some(Item::Value(old)) => {
            if old.to_string().trim() == new.to_string().trim() {
                return false;
            }
            let decor = old.decor().clone();
            *new.decor_mut() = decor;
            *old = new;
        }
        _ => {
            table.insert(key, Item::Value(new));
        }
    }
    true
}

/// Removes `section.key`, then the table itself when that left it empty; the comments above
/// (or beside) a removed table's header move to the next table, or to the end of the file.
fn remove(document: &mut DocumentMut, section: &str, key: &str) -> bool {
    let Some(table) = document.get_mut(section).and_then(Item::as_table_like_mut) else {
        return false;
    };
    if table.remove(key).is_none() {
        return false;
    }
    if table.is_empty() {
        let (comments, position) = match document.get(section) {
            Some(Item::Table(table)) => (comments(table.decor()), table.position()),
            _ => (String::new(), None),
        };
        document.remove(section);
        if !comments.is_empty() {
            carry(document, position, &comments);
        }
    }
    true
}

/// The text of a raw string of the document (empty when it has none).
fn raw(text: Option<&toml_edit::RawString>) -> String {
    text.and_then(toml_edit::RawString::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// The comment lines in a table's decoration: the lines above its header with their blank
/// lines, and a comment after the header on a line of its own.
fn comments(decor: &toml_edit::Decor) -> String {
    let mut kept = String::new();
    let prefix = raw(decor.prefix());
    if prefix.contains('#') {
        kept.push_str(&prefix);
    }
    let suffix = raw(decor.suffix());
    if suffix.contains('#') {
        kept.push_str(suffix.trim());
        kept.push('\n');
    }
    kept
}

/// Puts `comments` above the first table after `position`, or at the end of the document.
fn carry(document: &mut DocumentMut, position: Option<isize>, comments: &str) {
    let next = document
        .iter()
        .filter_map(|(key, item)| match item {
            Item::Table(table) if table.position() > position => {
                Some((table.position(), key.to_owned()))
            }
            _ => None,
        })
        .min();
    if let Some(Item::Table(table)) = next.and_then(|(_, key)| document.get_mut(&key)) {
        let decor = table.decor_mut();
        let prefix = raw(decor.prefix());
        decor.set_prefix(format!("{comments}{prefix}"));
        return;
    }
    let trailing = raw(Some(document.trailing()));
    document.set_trailing(format!("{trailing}{comments}"));
}

/// The `toml_edit` form of a configuration value (strings, integers, booleans and arrays of
/// them are all the configuration uses).
fn edit_value(value: &toml::Value) -> Option<toml_edit::Value> {
    Some(match value {
        toml::Value::String(text) => toml_edit::Value::from(text.as_str()),
        toml::Value::Integer(number) => toml_edit::Value::from(*number),
        toml::Value::Float(number) => toml_edit::Value::from(*number),
        toml::Value::Boolean(flag) => toml_edit::Value::from(*flag),
        toml::Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push_formatted(edit_value(item)?);
            }
            array.fmt();
            toml_edit::Value::Array(array)
        }
        toml::Value::Datetime(_) | toml::Value::Table(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str) -> DocumentMut {
        text.parse().unwrap()
    }

    /// Applies `change` to the configuration `text` holds and returns the patched text.
    fn patched(text: &str, change: impl FnOnce(&mut Config)) -> String {
        let mut document = doc(text);
        let before = Config::from_toml(text).unwrap();
        let mut after = before.clone();
        change(&mut after);
        assert!(patch(&mut document, &before, &after));
        let result = document.to_string();
        assert_eq!(Config::from_toml(&result).unwrap(), after);
        result
    }

    #[test]
    fn flatten_and_unflatten_round_trip() {
        let mut config = Config::default();
        config.hints.alphabet = "fjdk".to_owned();
        config.overlay.border = Some("#112233".to_owned());
        let keys = flatten(&config);
        assert_eq!(keys["hints.alphabet"], toml::Value::from("fjdk"));
        assert_eq!(keys["overlay.border"], toml::Value::from("#112233"));
        assert!(!keys.contains_key("overlay.background"));
        assert_eq!(unflatten(&keys).unwrap(), config);
    }

    #[test]
    fn unflatten_validates() {
        let mut keys = flatten(&Config::default());
        keys.insert("scroll.step".to_owned(), toml::Value::Integer(0));
        assert!(matches!(
            unflatten(&keys),
            Err(ConfigError::OutOfRange {
                field: "scroll.step",
                ..
            })
        ));
    }

    #[test]
    fn patch_sets_in_place_and_keeps_trailing_comments() {
        let text = patched(
            "[overlay]\nfont_size = 15 # big\ntheme = \"dark\"\n",
            |config| config.overlay.font_size = 18,
        );
        assert_eq!(text, "[overlay]\nfont_size = 18 # big\ntheme = \"dark\"\n");
    }

    #[test]
    fn patch_removes_values_back_at_their_default_and_empty_tables_keeping_comments() {
        let text = patched(
            "# mine\n\n[scroll] # fast\nstep = 5\n\n[hints]\nkeys = \"homerow\"\n",
            |config| config.scroll.step = Config::default().scroll.step,
        );
        assert_eq!(text, "# mine\n\n# fast\n\n[hints]\nkeys = \"homerow\"\n");
        let last = patched(
            "[hints]\nkeys = \"homerow\"\n\n# end\n[scroll]\nstep = 5\n",
            |c| {
                c.scroll.step = 3;
            },
        );
        assert_eq!(last, "[hints]\nkeys = \"homerow\"\n\n# end\n");
    }

    #[test]
    fn patch_adds_a_missing_table() {
        let text = patched("", |config| config.ignore.apps = vec!["kitty".to_owned()]);
        assert_eq!(text, "[ignore]\napps = [\"kitty\"]\n");
    }

    #[test]
    fn patch_writes_into_inline_and_dotted_tables() {
        let text = patched(
            "hints = { alphabet = \"fjdk\" }\nscroll.step = 4\n",
            |config| {
                config.hints.alphabet = "asdf".to_owned();
                config.scroll.step = 9;
            },
        );
        assert_eq!(text, "hints = { alphabet = \"asdf\" }\nscroll.step = 9\n");
    }

    #[test]
    fn unchanged_keys_stay_even_at_their_default() {
        let text = patched("[overlay]\nfont_size = 13\n", |config| {
            config.overlay.opacity = 80;
        });
        assert_eq!(text, "[overlay]\nfont_size = 13\nopacity = 80\n");
    }

    #[test]
    fn unsetting_an_optional_colour_removes_it() {
        let text = patched(
            "[overlay]\nborder = \"#112233\"\ntheme = \"dark\"\n",
            |config| {
                config.overlay.border = None;
            },
        );
        assert_eq!(text, "[overlay]\ntheme = \"dark\"\n");
    }

    #[test]
    fn clear_keeps_comments_and_unknown_keys() {
        let mut document =
            doc("# top\n[hints]\n# letters\nalphabet = \"fjdk\"\nfuture = 1\n[scroll]\nstep = 4\n");
        assert!(clear(&mut document));
        assert_eq!(document.to_string(), "# top\n[hints]\nfuture = 1\n");
        assert!(!clear(&mut document));
    }
}
