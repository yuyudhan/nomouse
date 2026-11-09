//! One control per settings field, chosen by its [`Kind`], titled and named by the field,
//! reading and writing its value through [`State`].

use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use nomouse_core::Config;
use nomouse_core::config::Appearance;
use nomouse_core::settings::{Choice, Field, Kind, Value};

use crate::list_editor::ListEditor;
use crate::recorder::Recorder;
use crate::state::State;
use crate::store::StoreError;
use crate::widgets;

/// A field's control.
#[derive(Clone)]
pub enum Control {
    /// [`Kind::Choice`].
    Choice(adw::ComboRow),
    /// [`Kind::Number`].
    Number(gtk::SpinButton),
    /// [`Kind::Toggle`].
    Toggle(gtk::Switch),
    /// [`Kind::Text`].
    Text(gtk::Entry),
    /// [`Kind::Hotkey`].
    Hotkey(Rc<Recorder>),
    /// [`Kind::Colour`]: the colour button and its "Use theme colour" reset.
    Colour(gtk::ColorButton, gtk::Button),
    /// [`Kind::Apps`].
    Apps(Rc<ListEditor>),
}

/// The system appearance `theme = "auto"` follows.
pub fn system_appearance() -> Appearance {
    if adw::StyleManager::default().is_dark() {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// The text of a [`Value::Text`], else empty.
pub fn text(value: &Value) -> &str {
    match value {
        Value::Text(text) => text,
        _ => "",
    }
}

/// `#rrggbb` for GDK's `rgb(r,g,b)` / `rgba(r,g,b,a)` text.
pub fn css_rgb_to_hex(text: &str) -> Option<String> {
    let inner = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))?
        .strip_suffix(')')?;
    let channels: Vec<u8> = inner
        .split(',')
        .take(3)
        .map(|part| part.trim().parse::<u8>().ok())
        .collect::<Option<_>>()?;
    match channels[..] {
        [r, g, b] => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        _ => None,
    }
}

/// The position of a colour key in [`nomouse_core::config::OverlayConfig::colours`].
pub fn colour_slot(key: &str) -> Option<usize> {
    [
        "overlay.background",
        "overlay.foreground",
        "overlay.typed_foreground",
        "overlay.border",
    ]
    .iter()
    .position(|slot| *slot == key)
}

/// The colour `key` has in effect: its own, else the theme's under the system appearance.
fn effective_colour(config: &Config, key: &str) -> Option<gdk::RGBA> {
    let appearance = system_appearance();
    let colours = config
        .overlay
        .colours(appearance)
        .unwrap_or_else(|_| config.overlay.theme.palette(appearance));
    let [r, g, b] = colours.get(colour_slot(key)?)?;
    Some(gdk::RGBA::new(
        f32::from(*r) / 255.0,
        f32::from(*g) / 255.0,
        f32::from(*b) / 255.0,
        1.0,
    ))
}

/// Builds `field`'s control and adds its rows to `group`. `editable_hotkeys` is false where
/// the compositor owns global shortcuts.
pub fn build(
    state: &Rc<State>,
    field: &'static Field,
    group: &adw::PreferencesGroup,
    editable_hotkeys: bool,
) -> Control {
    match field.kind {
        Kind::Choice(choices) => Control::Choice(choice(state, field, choices, group)),
        Kind::Number { min, max } => Control::Number(number(state, field, min, max, group)),
        Kind::Toggle => Control::Toggle(toggle(state, field, group)),
        Kind::Text => Control::Text(entry(state, field, group)),
        Kind::Hotkey => {
            let recorder = Recorder::new(state, field, editable_hotkeys);
            group.add(recorder.row());
            Control::Hotkey(recorder)
        }
        Kind::Colour => {
            let (button, reset) = colour(state, field, group);
            Control::Colour(button, reset)
        }
        Kind::Apps => {
            let editor = ListEditor::new(state, field);
            group.add(editor.list());
            group.add(editor.adder());
            Control::Apps(editor)
        }
    }
}

fn choice(
    state: &Rc<State>,
    field: &'static Field,
    choices: &'static [Choice],
    group: &adw::PreferencesGroup,
) -> adw::ComboRow {
    let titles: Vec<&str> = choices.iter().map(|choice| choice.title).collect();
    let row = widgets::combo_row(field.title, field.help, &titles);
    group.add(&row);
    let edit = Rc::clone(state);
    row.connect_selected_notify(move |row| {
        let picked = usize::try_from(row.selected())
            .ok()
            .and_then(|index| choices.get(index));
        if let Some(choice) = picked {
            edit.set(field.key, Value::Text(choice.value.to_owned()))
                .ok();
        }
    });
    let shown = row.clone();
    let read = Rc::clone(state);
    state.connect_refresh(move |_| {
        let value = read.value(field.key);
        let index = choices
            .iter()
            .position(|choice| choice.value == text(&value))
            .and_then(|index| u32::try_from(index).ok())
            .unwrap_or(0);
        shown.set_selected(index);
    });
    row
}

fn number(
    state: &Rc<State>,
    field: &'static Field,
    min: u32,
    max: u32,
    group: &adw::PreferencesGroup,
) -> gtk::SpinButton {
    let subtitle = if field.help.is_empty() {
        format!("{min} to {max}")
    } else {
        format!("{} {min} to {max}.", field.help)
    };
    let (row, spin) = widgets::spin_row(field.title, &subtitle, min, max);
    group.add(&row);
    let edit = Rc::clone(state);
    spin.connect_value_changed(move |spin| {
        edit.set(field.key, Value::Number(widgets::spin_value(spin)))
            .ok();
    });
    let shown = spin.clone();
    let read = Rc::clone(state);
    state.connect_refresh(move |_| {
        if let Value::Number(number) = read.value(field.key) {
            shown.set_value(f64::from(number));
        }
    });
    spin
}

fn toggle(state: &Rc<State>, field: &'static Field, group: &adw::PreferencesGroup) -> gtk::Switch {
    let (row, switch) = widgets::switch_row(field.title, field.help);
    group.add(&row);
    let edit = Rc::clone(state);
    switch.connect_active_notify(move |switch| {
        edit.set(field.key, Value::Bool(switch.is_active())).ok();
    });
    let shown = switch.clone();
    let read = Rc::clone(state);
    state.connect_refresh(move |_| {
        shown.set_active(read.value(field.key) == Value::Bool(true));
    });
    switch
}

fn entry(state: &Rc<State>, field: &'static Field, group: &adw::PreferencesGroup) -> gtk::Entry {
    let row = widgets::row(field.title, field.help);
    let entry: gtk::Entry = widgets::named(field.title);
    entry.set_valign(gtk::Align::Center);
    entry.set_width_chars(16);
    row.add_suffix(&entry);
    row.set_activatable_widget(Some(&entry));
    group.add(&row);
    let edit = Rc::clone(state);
    let message = row.clone();
    entry.connect_changed(move |entry| {
        if edit.is_refreshing() {
            return;
        }
        let result = edit.set(field.key, Value::Text(entry.text().to_string()));
        let error = match &result {
            Err(StoreError::Rejected(error)) => Some(error.as_str()),
            _ => None,
        };
        widgets::set_error(entry, error.is_some());
        widgets::set_subtitle(&message, error.unwrap_or(field.help));
        widgets::set_subtitle_error(&message, error.is_some());
    });
    let shown = entry.clone();
    let read = Rc::clone(state);
    let message = row;
    state.connect_refresh(move |_| {
        shown.set_text(text(&read.value(field.key)));
        widgets::set_error(&shown, false);
        widgets::set_subtitle(&message, field.help);
        widgets::set_subtitle_error(&message, false);
    });
    entry
}

fn colour(
    state: &Rc<State>,
    field: &'static Field,
    group: &adw::PreferencesGroup,
) -> (gtk::ColorButton, gtk::Button) {
    let row = widgets::row(field.title, "");
    let button: gtk::ColorButton = widgets::named(field.title);
    button.set_use_alpha(false);
    button.set_title(field.title);
    button.set_valign(gtk::Align::Center);
    let reset = widgets::button(
        "Use theme colour",
        &format!("{}: use theme colour", field.title),
    );
    reset.add_css_class("flat");
    row.add_suffix(&reset);
    row.add_suffix(&button);
    group.add(&row);
    let edit = Rc::clone(state);
    button.connect_color_set(move |button| {
        if let Some(hex) = css_rgb_to_hex(&button.rgba().to_string()) {
            edit.set(field.key, Value::Text(hex)).ok();
        }
    });
    let edit = Rc::clone(state);
    reset.connect_clicked(move |_| {
        edit.set(field.key, Value::Unset).ok();
    });
    // The effective colour follows the theme (and the system appearance under Auto), so it
    // is shown on every change, not only when the file changes.
    let (shown, shown_reset) = (button.clone(), reset.clone());
    let read = Rc::clone(state);
    let show = move |config: &Config| {
        let value = read.value(field.key);
        if let Some(rgba) = effective_colour(config, field.key) {
            shown.set_rgba(&rgba);
        }
        shown_reset.set_sensitive(value != Value::Unset);
        // The colour's own `#rrggbb`, or where it comes from (the shared help only says that).
        let own = text(&value);
        widgets::set_subtitle(&row, if own.is_empty() { "Theme colour" } else { own });
    };
    let on_dark = show.clone();
    let follow = Rc::clone(state);
    adw::StyleManager::default().connect_dark_notify(move |_| on_dark(&follow.config()));
    state.connect_changed(show);
    (button, reset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_gdk_colours_to_hex() {
        assert_eq!(css_rgb_to_hex("rgb(255,0,16)"), Some("#ff0010".to_owned()));
        assert_eq!(
            css_rgb_to_hex("rgba(1,2,3,0.5)"),
            Some("#010203".to_owned())
        );
        assert_eq!(css_rgb_to_hex("rgb(256,0,0)"), None);
        assert_eq!(css_rgb_to_hex("#ffffff"), None);
        assert_eq!(css_rgb_to_hex("rgb(1,2)"), None);
    }

    #[test]
    fn every_colour_field_has_a_slot() {
        for field in nomouse_core::settings::FIELDS {
            assert_eq!(
                colour_slot(field.key).is_some(),
                field.kind == Kind::Colour,
                "{}",
                field.key
            );
        }
    }
}
