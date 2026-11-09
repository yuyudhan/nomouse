//! A settings section's page, built from the shared field list
//! ([`nomouse_core::settings::Section::fields`]) so it matches the macOS window: one control per
//! field that applies on Linux, plus the Linux window's extras for some sections.

use std::rc::Rc;

use adw::prelude::*;
use nomouse_core::Hotkey;
use nomouse_core::settings::{Field, Section};

use crate::args::Page;
use crate::controls::{self, Control, text};
use crate::pages::advanced::FileGroup;
use crate::pages::appearance::Preview;
use crate::pages::hotkeys;
use crate::state::State;
use crate::widgets;

/// The OS the window runs on, as [`Field::applies_on`] names it.
const OS: &str = "linux";

/// A section's page.
pub struct SectionPage {
    /// The page.
    pub page: adw::PreferencesPage,
    /// The fields shown, each with its control.
    pub controls: Vec<(&'static Field, Control)>,
    /// Appearance only: the live preview.
    pub preview: Option<Rc<Preview>>,
    /// Advanced only: the config file group.
    pub file: Option<FileGroup>,
}

impl SectionPage {
    /// Builds the page of `section`.
    pub fn new(state: &Rc<State>, section: Section) -> Self {
        let page = widgets::page(Page::Settings(section));
        let preview = (section == Section::Appearance).then(|| Preview::new(state));
        if let Some(preview) = &preview {
            page.add(&preview.group);
        }
        let group = widgets::group("", section.summary());
        let editable_hotkeys = state.shortcuts.grabs_keys();
        let controls = section
            .fields()
            .filter(|field| field.applies_on(OS))
            .map(|field| {
                (
                    field,
                    controls::build(state, field, &group, editable_hotkeys),
                )
            })
            .collect();
        page.add(&group);
        if section == Section::Hotkeys && !editable_hotkeys {
            page.add(&hotkeys::instructions(state));
        }
        if section == Section::Search {
            page.add(&search_shortcut(state));
        }
        let file = (section == Section::Advanced).then(|| FileGroup::new(state));
        if let Some(file) = &file {
            page.add(&file.group);
        }
        Self {
            page,
            controls,
            preview,
            file,
        }
    }

    /// The control of `key`.
    pub fn control(&self, key: &str) -> Option<&Control> {
        self.controls
            .iter()
            .find(|(field, _)| field.key == key)
            .map(|(_, control)| control)
    }
}

/// Search: the search hotkey, with a way to the Hotkeys page.
fn search_shortcut(state: &Rc<State>) -> adw::PreferencesGroup {
    let group = widgets::group("Search hotkey", "");
    let row = widgets::row("Search hotkey", "");
    let change = widgets::button("Change…", "Change search hotkey");
    change.add_css_class("flat");
    row.add_suffix(&change);
    group.add(&row);
    let navigate = Rc::clone(state);
    change.connect_clicked(move |_| navigate.navigate(Page::Settings(Section::Hotkeys)));
    let read = Rc::clone(state);
    state.connect_changed(move |_| {
        let value = read.value("hotkeys.search");
        let shown = text(&value)
            .parse::<Hotkey>()
            .map_or_else(|_| text(&value).to_owned(), |hotkey| hotkey.human());
        widgets::set_subtitle(&row, &shown);
    });
    group
}
