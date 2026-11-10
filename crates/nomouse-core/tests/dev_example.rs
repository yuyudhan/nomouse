//! `config/dev.example.toml`, which `just dev-config` installs for a development build: it must
//! set every key the settings model knows, load, and behave exactly as a development build
//! without a config file.

#[cfg(test)]
mod dev_example {
    use std::path::Path;

    use nomouse_core::config::{Appearance, Config, HotkeysConfig, Theme};
    use nomouse_core::{Flavour, settings::FIELDS};

    fn example() -> (String, Config) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/dev.example.toml");
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let config = Config::load(&path).unwrap_or_else(|error| panic!("{error}"));
        (text, config)
    }

    #[test]
    fn it_sets_exactly_the_keys_the_settings_model_knows() {
        let (text, _) = example();
        let table: toml::Table = text.parse().expect("TOML");
        let mut set: Vec<String> = table
            .iter()
            .flat_map(|(section, keys)| {
                let keys = keys.as_table().expect("every section is a table");
                keys.keys().map(move |key| format!("{section}.{key}"))
            })
            .collect();
        set.sort();
        let mut every: Vec<String> = FIELDS.iter().map(|field| field.key.to_owned()).collect();
        every.sort();
        assert_eq!(set, every);
    }

    #[test]
    fn it_behaves_as_a_development_build_without_a_file() {
        let (_, config) = example();
        let defaults = Config {
            hotkeys: HotkeysConfig::defaults(Flavour::Dev),
            ..Config::default()
        };
        let mut colourless = config.clone();
        for colour in [
            &mut colourless.overlay.background,
            &mut colourless.overlay.foreground,
            &mut colourless.overlay.typed_foreground,
            &mut colourless.overlay.border,
        ] {
            *colour = None;
        }
        assert_eq!(colourless, defaults, "every other key is at its default");
        assert_eq!(config.overlay.theme, Theme::Original);
        for appearance in [Appearance::Light, Appearance::Dark] {
            assert_eq!(
                config.overlay.colours(appearance).unwrap(),
                defaults.overlay.colours(appearance).unwrap(),
                "the four colours are the original theme's"
            );
        }
    }
}
