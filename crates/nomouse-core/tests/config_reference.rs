//! Keeps `docs/configuration.md` in step with [`Config`]: its tables must list exactly the keys
//! the configuration serialises, each with the default the code uses, and every TOML example in
//! the README and the user docs must load.

#[cfg(test)]
mod reference {
    use std::collections::BTreeMap;
    use std::path::Path;

    use nomouse_core::config::Config;

    /// A file of the repository, by its path from the root.
    fn read(relative: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
    }

    /// Every `section.key` of `config` with its value written as TOML.
    fn keys(config: &Config) -> BTreeMap<String, String> {
        let value = toml::Value::try_from(config).expect("the configuration serialises");
        let mut keys = BTreeMap::new();
        for (section, table) in value.as_table().expect("a table at the top") {
            for (key, value) in table.as_table().expect("every section is a table") {
                keys.insert(format!("{section}.{key}"), value.to_string());
            }
        }
        keys
    }

    /// The documented keys with their Default column: rows like
    /// ``| `hints.alphabet` | `"asdfghjk"` | ... |``. A default of `unset` stays as it is.
    fn documented(page: &str) -> BTreeMap<String, String> {
        let mut rows = BTreeMap::new();
        for line in page.lines() {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let [_, key, default, ..] = cells.as_slice() else {
                continue;
            };
            let Some(key) = key.strip_prefix('`').and_then(|k| k.strip_suffix('`')) else {
                continue;
            };
            if !key.contains('.') {
                continue;
            }
            let default = default
                .strip_prefix('`')
                .and_then(|d| d.strip_suffix('`'))
                .unwrap_or(default);
            let previous = rows.insert(key.to_owned(), default.to_owned());
            assert!(previous.is_none(), "{key} is documented twice");
        }
        rows
    }

    /// The configuration with every optional key set, so each one serialises.
    fn every_key_set() -> Config {
        let mut config = Config::default();
        let colour = Some("#000000".to_owned());
        config.overlay.background.clone_from(&colour);
        config.overlay.foreground.clone_from(&colour);
        config.overlay.typed_foreground.clone_from(&colour);
        config.overlay.border = colour;
        config
    }

    #[test]
    fn the_page_lists_every_key_with_its_default() {
        let defaults = keys(&Config::default());
        let expected: BTreeMap<String, String> = keys(&every_key_set())
            .into_keys()
            .map(|key| {
                let default = defaults
                    .get(&key)
                    .cloned()
                    .unwrap_or_else(|| "unset".to_owned());
                (key, default)
            })
            .collect();
        let documented = documented(&read("docs/configuration.md"));
        for (key, default) in &expected {
            match documented.get(key) {
                None => panic!("docs/configuration.md does not document {key} (default {default})"),
                Some(written) => assert_eq!(
                    written, default,
                    "docs/configuration.md gives {key} the default {written}, the code {default}"
                ),
            }
        }
        for key in documented.keys() {
            assert!(
                expected.contains_key(key),
                "docs/configuration.md documents {key}, which the configuration does not have"
            );
        }
    }

    #[test]
    fn every_example_in_the_docs_loads() {
        for (file, at_least) in [
            ("README.md", 1),
            ("docs/configuration.md", 1),
            ("docs/troubleshooting.md", 1),
            ("docs/usage.md", 0),
        ] {
            let text = read(file);
            let mut examples = 0;
            let mut rest = text.as_str();
            while let Some(start) = rest.find("```toml\n") {
                let body = &rest[start + "```toml\n".len()..];
                let end = body.find("```").expect("a closed code block");
                Config::from_toml(&body[..end]).unwrap_or_else(|error| {
                    panic!("{file}: example {} does not load: {error}", examples + 1)
                });
                examples += 1;
                rest = &body[end..];
            }
            assert!(
                examples >= at_least,
                "{file} has {examples} TOML examples, expected at least {at_least}"
            );
        }
    }
}
