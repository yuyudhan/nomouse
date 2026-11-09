//! The hotkey recorder: a button showing a mode's chord; clicking it records the next chord.
//!
//! While it records, the daemon is paused (its X11 grabs would swallow the chord) and resumed
//! afterwards if it was running before.

use std::cell::Cell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;
use nomouse_core::settings::{Field, Value};
use nomouse_core::{Hotkey, Modifiers};

use crate::controls::text;
use crate::keymap::{Recorded, hotkey_from_key};
use crate::state::State;
use crate::store::StoreError;
use crate::widgets;

/// What the button says while recording.
pub const PROMPT: &str = "Press a shortcut… Esc cancels";

/// One hotkey field's recorder row.
pub struct Recorder {
    field: &'static Field,
    state: Rc<State>,
    row: adw::ActionRow,
    button: gtk::Button,
    recording: Cell<bool>,
}

impl Recorder {
    /// Builds the row; `editable` is false where the compositor owns the shortcuts.
    pub fn new(state: &Rc<State>, field: &'static Field, editable: bool) -> Rc<Self> {
        let row = widgets::row(field.title, field.help);
        let button = widgets::button("", field.title);
        button.set_sensitive(editable);
        button.set_width_request(180);
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));
        let recorder = Rc::new(Self {
            field,
            state: Rc::clone(state),
            row,
            button,
            recording: Cell::new(false),
        });
        let weak = Rc::downgrade(&recorder);
        recorder.button.connect_clicked(move |_| {
            if let Some(recorder) = weak.upgrade()
                && !recorder.recording.get()
            {
                recorder.start();
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&recorder);
        keys.connect_key_pressed(move |controller, keyval, keycode, modifiers| {
            match weak.upgrade() {
                Some(recorder) if recorder.recording.get() => {
                    let name = unshifted_name(controller, keyval, keycode);
                    recorder.press(&name, modifiers_of(modifiers));
                    gtk::glib::Propagation::Stop
                }
                _ => gtk::glib::Propagation::Proceed,
            }
        });
        recorder.button.add_controller(keys);
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(&recorder);
        focus.connect_leave(move |_| {
            if let Some(recorder) = weak.upgrade() {
                recorder.stop();
            }
        });
        recorder.button.add_controller(focus);
        let weak = Rc::downgrade(&recorder);
        state.connect_refresh(move |_| {
            if let Some(recorder) = weak.upgrade() {
                recorder.set_message(None);
                recorder.show();
            }
        });
        recorder
    }

    /// The field this recorder edits.
    pub fn field(&self) -> &'static Field {
        self.field
    }

    /// The row.
    pub fn row(&self) -> &adw::ActionRow {
        &self.row
    }

    /// The recorder button.
    pub fn button(&self) -> &gtk::Button {
        &self.button
    }

    /// Whether the next key press is recorded.
    pub fn is_recording(&self) -> bool {
        self.recording.get()
    }

    /// Starts recording: the button prompts and takes the keyboard focus.
    pub fn start(&self) {
        self.recording.set(true);
        self.state.daemon.begin_recording();
        self.button.set_label(PROMPT);
        self.button.add_css_class("suggested-action");
        self.set_message(None);
        self.button.grab_focus();
    }

    /// Stops recording and shows the saved chord again.
    pub fn stop(&self) {
        if self.recording.replace(false) {
            self.state.daemon.end_recording();
            self.button.remove_css_class("suggested-action");
            self.show();
        }
    }

    /// Handles a key press while recording: `key` is the GDK name of the key without
    /// modifiers (`slash`, `a`, `F5`, `Escape`). Returns what happened to the hotkey, or
    /// `None` while waiting for a key after a modifier (or when not recording).
    pub fn press(&self, key: &str, modifiers: Modifiers) -> Option<Result<(), String>> {
        if !self.recording.get() {
            return None;
        }
        let recorded = hotkey_from_key(key, modifiers);
        let result = match &recorded {
            Recorded::Pending => return None,
            Recorded::Hotkey(hotkey) => {
                self.stop();
                self.apply(*hotkey)
            }
            Recorded::Cancel => {
                self.stop();
                Ok(())
            }
            Recorded::NeedsModifier(_) | Recorded::Unsupported(_) => {
                // Keep recording so the next try needs no click.
                let message = recorded.message().unwrap_or_default();
                self.set_message(Some(&message));
                Err(message)
            }
        };
        Some(result)
    }

    /// Saves `hotkey` for this field; refused (and shown) when it is invalid or another mode
    /// has it.
    ///
    /// # Errors
    /// Why the hotkey was refused.
    pub fn apply(&self, hotkey: Hotkey) -> Result<(), String> {
        let result = self
            .state
            .set(self.field.key, Value::Text(hotkey.to_string()));
        self.show();
        match result {
            Ok(()) => {
                self.set_message(None);
                Ok(())
            }
            Err(error) => {
                let text = match error {
                    StoreError::Rejected(reason) => {
                        format!("{} was not saved: {reason}", hotkey.human())
                    }
                    StoreError::Failed(reason) => reason,
                };
                self.set_message(Some(&text));
                Err(text)
            }
        }
    }

    /// The row subtitle: the error in red, else the field's help.
    fn set_message(&self, error: Option<&str>) {
        widgets::set_subtitle(&self.row, error.unwrap_or(self.field.help));
        widgets::set_subtitle_error(&self.row, error.is_some());
    }

    fn show(&self) {
        if !self.recording.get() {
            let value = self.state.value(self.field.key);
            let label = text(&value)
                .parse::<Hotkey>()
                .map_or_else(|_| text(&value).to_owned(), |hotkey| hotkey.human());
            self.button.set_label(&label);
        }
    }
}

/// The GDK name of the key at `keycode` without any modifier, so Shift+/ is `slash`.
fn unshifted_name(controller: &gtk::EventControllerKey, keyval: gdk::Key, keycode: u32) -> String {
    let group = i32::try_from(controller.group()).unwrap_or(0);
    let translated = controller
        .widget()
        .map(|widget| widget.display())
        .and_then(|display| display.translate_key(keycode, gdk::ModifierType::empty(), group))
        .map_or(keyval, |(key, ..)| key);
    translated
        .name()
        .or_else(|| keyval.name())
        .map(|name| name.to_string())
        .unwrap_or_default()
}

/// The nomouse modifiers in a GDK modifier state.
pub fn modifiers_of(state: gdk::ModifierType) -> Modifiers {
    Modifiers {
        ctrl: state.contains(gdk::ModifierType::CONTROL_MASK),
        alt: state.contains(gdk::ModifierType::ALT_MASK),
        shift: state.contains(gdk::ModifierType::SHIFT_MASK),
        meta: state.contains(gdk::ModifierType::SUPER_MASK),
    }
}
