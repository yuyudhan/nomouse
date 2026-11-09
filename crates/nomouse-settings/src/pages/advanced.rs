//! The Advanced page's extra: the config file (open it, reset it).

use std::rc::Rc;

use adw::prelude::*;

use crate::state::State;
use crate::widgets;

/// The config file group.
pub struct FileGroup {
    /// The group.
    pub group: adw::PreferencesGroup,
    /// "Open config file".
    pub open: gtk::Button,
    /// "Reset to defaults…".
    pub reset: gtk::Button,
}

impl FileGroup {
    /// Builds the group.
    pub fn new(state: &Rc<State>) -> Self {
        let group = widgets::group("Config file", "");
        let path = state.path().display().to_string();
        let file_row = widgets::row("Config file", &path);
        let open = widgets::button("Open", "Open config file");
        file_row.add_suffix(&open);
        group.add(&file_row);
        let reset_row = widgets::row(
            "Reset to defaults…",
            "Every setting goes back to its default; comments in the file stay",
        );
        let reset = widgets::button("Reset…", "Reset to defaults…");
        reset.add_css_class("destructive-action");
        reset_row.add_suffix(&reset);
        group.add(&reset_row);
        let opener = Rc::clone(state);
        open.connect_clicked(move |_| opener.open_config_file());
        let resetter = Rc::clone(state);
        reset.connect_clicked(move |button| {
            let parent = button.root().and_downcast::<gtk::Window>();
            confirm_reset(parent.as_ref(), &resetter);
        });
        Self { group, open, reset }
    }
}

/// Asks before resetting: a modal dialog with Cancel and a destructive Reset.
pub fn confirm_reset(parent: Option<&gtk::Window>, state: &Rc<State>) -> gtk::MessageDialog {
    let dialog = gtk::MessageDialog::builder()
        .modal(true)
        .message_type(gtk::MessageType::Question)
        .text("Reset all settings?")
        .secondary_text("Every setting goes back to its default. Comments in the file stay.")
        .build();
    dialog.set_transient_for(parent);
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    if let Some(reset) = dialog
        .add_button("Reset", gtk::ResponseType::Accept)
        .downcast_ref::<gtk::Button>()
    {
        reset.add_css_class("destructive-action");
    }
    dialog.set_default_response(gtk::ResponseType::Cancel);
    let state = Rc::clone(state);
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept {
            state.reset();
        }
        dialog.close();
    });
    dialog.present();
    dialog
}
