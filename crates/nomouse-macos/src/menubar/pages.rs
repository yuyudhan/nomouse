//! The settings window's pages: General (permission, login item, updates), then one page per
//! [`Section`] with a control for every field that applies on macOS, in
//! [`nomouse_core::settings::FIELDS`] order. Each field's control reports with
//! [`field_tag`]; the window's own controls use the tags below.

use nomouse_core::settings::{FIELDS, Field, Kind, Section};
use nomouse_sys_macos::ui::form::{Page, Row, Spec};

/// The OS the window shows fields for.
const OS: &str = "macos";

/// Tag of the first field; field `i` of [`FIELDS`] reports `FIELD_TAG + i`. Below it are the
/// window's own controls, above the menu's action ids (1 to 14) and its Accessibility line.
const FIELD_TAG: isize = 100;

/// The label preview on the Appearance page.
pub(super) const PREVIEW: isize = 60;
/// The Accessibility status line on the General page.
pub(super) const ACCESS_NOTE: isize = 61;
/// "Open Accessibility Settings".
pub(super) const ACCESS_BUTTON: isize = 62;
/// The start at login checkbox.
pub(super) const LOGIN: isize = 63;
/// The line below it (needs approval, unavailable).
pub(super) const LOGIN_NOTE: isize = 64;
/// The update check's answer.
pub(super) const UPDATE_NOTE: isize = 65;
/// "Check for Updates".
pub(super) const UPDATE_BUTTON: isize = 66;
/// "Run Doctor".
pub(super) const DOCTOR: isize = 67;
/// "About nomouse".
pub(super) const ABOUT: isize = 68;
/// The config file's path on the Advanced page.
pub(super) const CONFIG_NOTE: isize = 69;
/// "Open Config File".
pub(super) const OPEN_CONFIG: isize = 70;
/// "Show in Finder".
pub(super) const REVEAL_CONFIG: isize = 71;
/// "Reset to Defaults…".
pub(super) const RESET_ALL: isize = 72;

/// The index of the General page; the section pages follow it in [`Section::ALL`] order.
pub(super) const GENERAL_PAGE: usize = 0;

/// The tag of `field`'s control.
pub(super) fn field_tag(field: &Field) -> isize {
    let index = FIELDS
        .iter()
        .position(|known| known.key == field.key)
        .unwrap_or(0);
    FIELD_TAG + isize::try_from(index).unwrap_or(0)
}

/// The field whose control reports `tag`, if it is one shown on macOS.
pub(super) fn field_at(tag: isize) -> Option<&'static Field> {
    let index = usize::try_from(tag.checked_sub(FIELD_TAG)?).ok()?;
    FIELDS.get(index).filter(|field| field.applies_on(OS))
}

/// The fields the window shows, in page order.
pub(super) fn shown_fields() -> impl Iterator<Item = &'static Field> {
    Section::ALL
        .into_iter()
        .flat_map(Section::fields)
        .filter(|field| field.applies_on(OS))
}

/// The page index of `section`.
pub(super) fn page_of(section: Section) -> usize {
    1 + Section::ALL
        .iter()
        .position(|known| *known == section)
        .unwrap_or(0)
}

const fn symbol(section: Section) -> &'static str {
    match section {
        Section::Hotkeys => "command",
        Section::Labels => "textformat.abc",
        Section::Appearance => "paintpalette",
        Section::Scroll => "scroll",
        Section::Search => "magnifyingglass",
        Section::IgnoredApps => "nosign",
        Section::Advanced => "slider.horizontal.3",
    }
}

fn spec(field: &Field) -> Spec {
    let tag = field_tag(field);
    match field.kind {
        Kind::Text => Spec::Text { tag },
        Kind::Choice(choices) => Spec::Choice {
            tag,
            titles: choices
                .iter()
                .map(|choice| choice.title.to_owned())
                .collect(),
        },
        Kind::Number { min, max } => Spec::Number {
            tag,
            min,
            max,
            step: 1,
        },
        Kind::Toggle => Spec::Toggle {
            tag,
            title: field.title.to_owned(),
        },
        Kind::Hotkey => Spec::Shortcut { tag },
        Kind::Colour => Spec::Colour { tag },
        Kind::Apps => Spec::Apps { tag },
    }
}

fn row(label: &str, spec: Spec, help: &str, id: &str) -> Row {
    Row {
        label: label.to_owned(),
        spec,
        help: help.to_owned(),
        id: id.to_owned(),
    }
}

fn general() -> Page {
    Page {
        title: "General".to_owned(),
        symbol: "gearshape".to_owned(),
        summary: "nomouse runs in the menu bar. Every change here is saved to config.toml at \
                  once and the hotkeys pick it up within a second."
            .to_owned(),
        rows: vec![
            row(
                "Accessibility:",
                Spec::Note { tag: ACCESS_NOTE },
                "",
                "general.accessibility",
            ),
            row(
                "",
                Spec::Buttons(vec![(
                    ACCESS_BUTTON,
                    "Open Accessibility Settings".to_owned(),
                )]),
                "nomouse reads and clicks controls through the accessibility API, which macOS \
                 allows only to apps you trust.",
                "",
            ),
            row(
                "Login:",
                Spec::Toggle {
                    tag: LOGIN,
                    title: "Start nomouse when you log in".to_owned(),
                },
                "",
                "general.start_at_login",
            ),
            row("", Spec::Note { tag: LOGIN_NOTE }, "", "general.login_note"),
            row(
                "Updates:",
                Spec::Note { tag: UPDATE_NOTE },
                "",
                "general.update",
            ),
            row(
                "",
                Spec::Buttons(vec![(UPDATE_BUTTON, "Check for Updates".to_owned())]),
                "Asks GitHub for the latest release; installing stays with the installer.",
                "",
            ),
            row(
                "Help:",
                Spec::Buttons(vec![
                    (DOCTOR, "Run Doctor".to_owned()),
                    (ABOUT, "About nomouse".to_owned()),
                ]),
                "The doctor checks the permission, the event tap and the config file.",
                "",
            ),
        ],
    }
}

fn section(section: Section) -> Page {
    let mut rows: Vec<Row> = section
        .fields()
        .filter(|field| field.applies_on(OS))
        .map(|field| {
            // A checkbox carries its title itself.
            let label = match field.kind {
                Kind::Toggle => String::new(),
                _ => format!("{}:", field.title),
            };
            row(&label, spec(field), field.help, field.key)
        })
        .collect();
    match section {
        Section::Appearance => rows.push(row(
            "Preview:",
            Spec::Preview { tag: PREVIEW },
            "",
            "appearance.preview",
        )),
        Section::Advanced => {
            rows.push(row(
                "Config file:",
                Spec::Note { tag: CONFIG_NOTE },
                "",
                "advanced.config_path",
            ));
            rows.push(row(
                "",
                Spec::Buttons(vec![
                    (OPEN_CONFIG, "Open Config File".to_owned()),
                    (REVEAL_CONFIG, "Show in Finder".to_owned()),
                    (RESET_ALL, "Reset to Defaults…".to_owned()),
                ]),
                "Edits made in a text editor show up here and in the hotkeys at once; a \
                 mistake is reported and the last good settings stay.",
                "",
            ));
        }
        Section::Hotkeys
        | Section::Labels
        | Section::Scroll
        | Section::Search
        | Section::IgnoredApps => {}
    }
    Page {
        title: section.title().to_owned(),
        symbol: symbol(section).to_owned(),
        summary: section.summary().to_owned(),
        rows,
    }
}

/// Every page of the window, General first.
pub(super) fn pages() -> Vec<Page> {
    std::iter::once(general())
        .chain(Section::ALL.into_iter().map(section))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn tags(spec: &Spec) -> Vec<isize> {
        match spec {
            Spec::Choice { tag, .. }
            | Spec::Text { tag }
            | Spec::Number { tag, .. }
            | Spec::Toggle { tag, .. }
            | Spec::Colour { tag }
            | Spec::Shortcut { tag }
            | Spec::Apps { tag }
            | Spec::Preview { tag }
            | Spec::Note { tag } => vec![*tag],
            Spec::Buttons(buttons) => buttons.iter().map(|(tag, _)| *tag).collect(),
        }
    }

    #[test]
    fn every_macos_field_has_one_control_and_tags_are_unique() {
        let pages = pages();
        assert_eq!(pages.len(), 1 + Section::ALL.len());
        let ids: BTreeSet<&str> = pages
            .iter()
            .flat_map(|page| &page.rows)
            .map(|row| row.id.as_str())
            .collect();
        for field in FIELDS {
            assert_eq!(
                ids.contains(field.key),
                field.applies_on("macos"),
                "{}",
                field.key
            );
        }
        let all: Vec<isize> = pages
            .iter()
            .flat_map(|page| &page.rows)
            .flat_map(|row| tags(&row.spec))
            .collect();
        let unique: BTreeSet<isize> = all.iter().copied().collect();
        assert_eq!(unique.len(), all.len(), "duplicate tags: {all:?}");
        // The menu's action ids (1 to 14) and its Accessibility line (50) stay free.
        assert!(all.iter().all(|tag| *tag > 50));
    }

    #[test]
    fn field_tags_round_trip_and_pages_follow_sections() {
        for field in shown_fields() {
            assert_eq!(field_at(field_tag(field)).map(|f| f.key), Some(field.key));
            assert_eq!(pages()[page_of(field.section)].title, field.section.title());
        }
        assert!(field_at(PREVIEW).is_none());
        assert_eq!(pages()[GENERAL_PAGE].title, "General");
    }
}
