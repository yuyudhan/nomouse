//! The Hotkeys page's extra where the compositor owns global shortcuts (Wayland): how to bind
//! the modes there, in a selectable monospace label with a Copy button.

use std::rc::Rc;

use adw::prelude::*;

use crate::state::State;
use crate::widgets;

/// The group with the compositor binding text.
pub fn instructions(state: &Rc<State>) -> adw::PreferencesGroup {
    let group = widgets::group(
        "Bind the modes in your desktop",
        "This session gives apps no global hotkeys, so nomouse cannot grab the keys above.",
    );
    let copy = widgets::button("Copy", "Copy");
    copy.add_css_class("flat");
    group.set_header_suffix(Some(&copy));
    let label = gtk::Label::builder()
        .selectable(true)
        .wrap(true)
        .xalign(0.0)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    label.add_css_class("monospace");
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("card");
    card.append(&label);
    group.add(&card);
    let shortcuts = state.shortcuts.clone();
    let shown = label.clone();
    state.connect_changed(move |config| {
        let text = shortcuts
            .instructions(&config.hotkeys)
            .or_else(|| shortcuts.warning())
            .unwrap_or_default();
        shown.set_text(&text);
    });
    let toast_state = Rc::clone(state);
    copy.connect_clicked(move |button| {
        button.clipboard().set_text(&label.text());
        toast_state.toast("Copied");
    });
    group
}
