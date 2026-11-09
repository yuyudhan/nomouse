use std::collections::BTreeSet;
use std::path::PathBuf;

use super::*;
use crate::config::Theme;

/// A fresh directory for one test, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("nomouse-settings-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("nomouse").join("config.toml")
    }

    fn write(&self, text: &str) {
        let file = self.file();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn read(&self) -> String {
        std::fs::read_to_string(self.file()).unwrap()
    }

    /// Every file in the config directory, so a test can see no temporary file is left.
    fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.0.join("nomouse"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[test]
fn a_missing_file_is_the_defaults_and_a_save_writes_only_what_differs() {
    let dir = Dir::new("round-trip");
    let mut settings = Settings::open(dir.file()).unwrap();
    assert_eq!(settings.config(), &Config::default());
    assert_eq!(settings.problem(), None);
    assert!(
        settings
            .update(|config| config.overlay.font_size = 18)
            .unwrap()
    );
    assert_eq!(dir.read(), "[overlay]\nfont_size = 18\n");
    assert_eq!(dir.names(), ["config.toml"]);

    let reopened = Settings::open(dir.file()).unwrap();
    assert_eq!(reopened.config(), settings.config());
    assert_eq!(reopened.config().overlay.font_size, 18);

    assert!(
        settings
            .update(|config| config.overlay.font_size = 13)
            .unwrap()
    );
    assert_eq!(dir.read(), "");
    assert!(!settings.update(|_| {}).unwrap());
}

#[test]
fn every_field_round_trips_through_the_file() {
    let dir = Dir::new("fields");
    let mut settings = Settings::open(dir.file()).unwrap();
    let values = [
        ("hotkeys.hints", Value::Text("shift+super+space".to_owned())),
        ("hints.alphabet", Value::Text("fjdksa".to_owned())),
        ("hints.keys", Value::Text("homerow".to_owned())),
        ("hints.browser", Value::Text("exhaustive".to_owned())),
        ("overlay.theme", Value::Text("dark".to_owned())),
        ("overlay.font_size", Value::Number(20)),
        ("overlay.border", Value::Text("#112233".to_owned())),
        ("scroll.dash_step", Value::Number(40)),
        ("hints.semantic_click", Value::Bool(true)),
        (
            "ignore.apps",
            Value::List(vec!["kitty".to_owned(), "com.apple.Terminal".to_owned()]),
        ),
    ];
    for (key, value) in &values {
        assert!(settings.set(key, value.clone()).unwrap(), "{key}");
        assert_eq!(&settings.value(key).unwrap(), value, "{key}");
    }
    // Hotkeys are written in their canonical spelling, whatever the window passed.
    assert!(
        settings
            .set("hotkeys.search", Value::Text("cmd+shift+/".to_owned()))
            .unwrap()
    );
    assert_eq!(
        settings.value("hotkeys.search").unwrap(),
        Value::Text("shift+super+slash".to_owned())
    );
    let reopened = Settings::open(dir.file()).unwrap();
    for (key, value) in &values {
        assert_eq!(&reopened.value(key).unwrap(), value, "{key}");
    }
    assert_eq!(reopened.config().overlay.theme, Theme::Dark);
    assert!(settings.set("overlay.border", Value::Unset).unwrap());
    assert!(!dir.read().contains("border"));
    assert_eq!(settings.value("overlay.border").unwrap(), Value::Unset);
}

#[test]
fn comments_order_and_formatting_survive_a_change() {
    let dir = Dir::new("comments");
    let original = "# My nomouse settings\n\n[hints]\n# home row first\nalphabet   =   \"fjdksa\"  # index fingers\nkeys = \"homerow\"\n\n# looks\n[overlay]\ntheme = \"light\" # daytime\nfont_size = 15\n";
    dir.write(original);
    let mut settings = Settings::open(dir.file()).unwrap();
    assert_eq!(settings.problem(), None);
    assert!(
        settings
            .set("overlay.theme", Value::Text("dark".to_owned()))
            .unwrap()
    );
    let dark = original.replace("\"light\"", "\"dark\"");
    assert_eq!(dir.read(), dark);
    assert!(settings.set("scroll.step", Value::Number(5)).unwrap());
    assert_eq!(dir.read(), format!("{dark}\n[scroll]\nstep = 5\n"));
}

#[test]
fn an_invalid_file_is_reported_and_never_overwritten() {
    let dir = Dir::new("invalid");
    dir.write("[overlay]\nfont_size = 500\n");
    let mut settings = Settings::open(dir.file()).unwrap();
    assert_eq!(settings.config(), &Config::default());
    assert!(
        settings
            .problem()
            .unwrap()
            .contains("overlay.font_size = 500")
    );

    dir.write("[overlay\nfont_size = 15\n");
    assert!(matches!(
        settings.update(|config| config.overlay.opacity = 50),
        Err(SettingsError::Syntax { .. })
    ));
    assert_eq!(dir.read(), "[overlay\nfont_size = 15\n");
    assert!(settings.problem().is_some());
}

#[test]
fn an_invalid_change_is_rejected_before_writing() {
    let dir = Dir::new("rejected");
    dir.write("[scroll]\nstep = 4\n");
    let mut settings = Settings::open(dir.file()).unwrap();
    assert!(matches!(
        settings.set("overlay.font_size", Value::Number(500)),
        Err(SettingsError::Rejected(ConfigError::OutOfRange { .. }))
    ));
    assert!(matches!(
        settings.set("hotkeys.scroll", Value::Text("ctrl+space".to_owned())),
        Err(SettingsError::Rejected(ConfigError::DuplicateHotkey { .. }))
    ));
    assert!(matches!(
        settings.update(|config| config.hints.alphabet = "ilo".to_owned()),
        Err(SettingsError::Rejected(_))
    ));
    assert!(matches!(
        settings.set("scroll.step", Value::Bool(true)),
        Err(SettingsError::WrongKind { .. })
    ));
    assert!(matches!(
        settings.set("scroll.speed", Value::Number(2)),
        Err(SettingsError::UnknownKey(_))
    ));
    assert_eq!(dir.read(), "[scroll]\nstep = 4\n");
    assert_eq!(settings.config().scroll.step, 4);
}

#[test]
fn keys_this_version_does_not_know_are_kept() {
    let dir = Dir::new("unknown");
    dir.write("[hints]\nkeys = \"homerow\"\n");
    let mut settings = Settings::open(dir.file()).unwrap();
    dir.write("[hints]\nkeys = \"homerow\"\nsparkle = true\n\n[future]\nanswer = 42\n");
    assert!(matches!(settings.refresh(), Change::Invalid(_)));
    assert_eq!(settings.config().hints.keys, crate::KeyBindings::Homerow);

    assert!(
        settings
            .set("overlay.theme", Value::Text("auto".to_owned()))
            .unwrap()
    );
    assert_eq!(
        dir.read(),
        "[hints]\nkeys = \"homerow\"\nsparkle = true\n\n[future]\nanswer = 42\n\n[overlay]\ntheme = \"auto\"\n"
    );
    // What was asked for is in effect for the window; the daemon still rejects the file.
    assert_eq!(settings.config().overlay.theme, Theme::Auto);
    assert!(settings.problem().unwrap().contains("unknown field"));

    assert!(settings.reset().unwrap());
    assert_eq!(
        dir.read(),
        "[hints]\nsparkle = true\n\n[future]\nanswer = 42\n"
    );
}

#[test]
fn edits_from_elsewhere_are_reloaded_and_bad_ones_keep_the_last_good() {
    let dir = Dir::new("watch");
    dir.write("[overlay]\ntheme = \"light\"\n");
    let mut settings = Settings::open(dir.file()).unwrap();
    assert!(matches!(settings.refresh(), Change::Unchanged));

    dir.write("[overlay]\ntheme = \"dark\"\n");
    assert!(matches!(settings.refresh(), Change::Reloaded));
    assert_eq!(settings.config().overlay.theme, Theme::Dark);

    dir.write("# just a comment now\n[overlay]\ntheme = \"dark\"\n");
    assert!(matches!(settings.refresh(), Change::Unchanged));

    dir.write("[overlay]\ntheme = \"purple\"\n");
    let Change::Invalid(error) = settings.refresh() else {
        panic!("an invalid edit must be reported");
    };
    assert!(error.to_string().contains("purple"), "{error}");
    assert_eq!(settings.config().overlay.theme, Theme::Dark);
    // Reported once per edit by refresh; again on an explicit reload.
    assert!(matches!(settings.refresh(), Change::Unchanged));
    assert!(matches!(settings.reload(), Change::Invalid(_)));

    dir.write("[overlay]\ntheme = \"auto\"\n");
    assert!(matches!(settings.refresh(), Change::Reloaded));
    assert_eq!(settings.problem(), None);

    std::fs::remove_file(dir.file()).unwrap();
    assert!(matches!(settings.refresh(), Change::Reloaded));
    assert_eq!(settings.config(), &Config::default());
}

#[test]
fn watch_looks_at_most_once_per_interval() {
    let dir = Dir::new("interval");
    let mut settings = Settings::open(dir.file()).unwrap();
    assert!(matches!(settings.watch(), Change::Unchanged));
    dir.write("[scroll]\nstep = 9\n");
    assert!(matches!(settings.watch(), Change::Unchanged));
    std::thread::sleep(WATCH_INTERVAL + Duration::from_millis(50));
    assert!(matches!(settings.watch(), Change::Reloaded));
    assert_eq!(settings.config().scroll.step, 9);
}

#[test]
fn a_change_reads_edits_made_since_the_last_look() {
    let dir = Dir::new("merge");
    let mut settings = Settings::open(dir.file()).unwrap();
    dir.write("[overlay]\ntheme = \"light\"\n");
    assert!(settings.set("scroll.step", Value::Number(6)).unwrap());
    assert_eq!(
        dir.read(),
        "[overlay]\ntheme = \"light\"\n\n[scroll]\nstep = 6\n"
    );
}

#[cfg(unix)]
#[test]
fn saving_follows_a_symlink_and_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = Dir::new("symlink");
    let dotfiles = dir.0.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    let real = dotfiles.join("nomouse.toml");
    std::fs::write(&real, "# mine\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::create_dir_all(dir.file().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, dir.file()).unwrap();

    let mut settings = Settings::open(dir.file()).unwrap();
    assert!(settings.set("scroll.step", Value::Number(7)).unwrap());
    assert!(
        std::fs::symlink_metadata(dir.file())
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "# mine\n[scroll]\nstep = 7\n"
    );
    assert_eq!(
        std::fs::metadata(&real).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let leftovers: Vec<_> = std::fs::read_dir(&dotfiles)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["nomouse.toml"]);
}

#[test]
fn the_fields_list_every_key_once_in_section_order() {
    let listed: Vec<&str> = FIELDS.iter().map(|field| field.key).collect();
    let unique: BTreeSet<String> = listed.iter().map(|key| (*key).to_owned()).collect();
    assert_eq!(unique.len(), listed.len(), "a key is listed twice");
    let configured: BTreeSet<String> = document::flatten(&document::every_key_set())
        .into_keys()
        .collect();
    assert_eq!(unique, configured);

    let order: Vec<Section> = FIELDS.iter().map(|field| field.section).collect();
    let mut sorted = order.clone();
    sorted.sort_by_key(|section| Section::ALL.iter().position(|s| s == section));
    assert_eq!(order, sorted, "fields are grouped in Section::ALL order");
    for section in Section::ALL {
        assert!(
            section.fields().next().is_some(),
            "{} is empty",
            section.title()
        );
    }
}

#[test]
fn every_choice_and_number_limit_matches_validation() {
    let dir = Dir::new("limits");
    let mut settings = Settings::open(dir.file()).unwrap();
    for field in FIELDS {
        match field.kind {
            Kind::Choice(choices) => {
                for choice in choices {
                    settings
                        .set(field.key, Value::Text(choice.value.to_owned()))
                        .unwrap_or_else(|error| {
                            panic!("{} = {}: {error}", field.key, choice.value)
                        });
                    assert_eq!(
                        settings.value(field.key).unwrap(),
                        Value::Text(choice.value.to_owned())
                    );
                }
            }
            Kind::Number { min, max } => {
                for good in [min, max] {
                    settings.set(field.key, Value::Number(good)).unwrap();
                }
                for bad in [min - 1, max + 1] {
                    assert!(
                        matches!(
                            settings.set(field.key, Value::Number(bad)),
                            Err(SettingsError::Rejected(_))
                        ),
                        "{} = {bad}",
                        field.key
                    );
                }
            }
            Kind::Text | Kind::Toggle | Kind::Hotkey | Kind::Colour | Kind::Apps => {}
        }
    }
}
