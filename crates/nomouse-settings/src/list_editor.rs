//! An editable list of app ids (a [`Kind::Apps`](nomouse_core::settings::Kind::Apps) field):
//! a row per entry with a remove button, and an entry with an Add button that refuses blanks
//! and duplicates.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use nomouse_core::settings::{Field, Value};

use crate::state::State;
use crate::widgets;

/// The editor.
pub struct ListEditor {
    state: Rc<State>,
    field: &'static Field,
    list: gtk::ListBox,
    adder: adw::ActionRow,
    entry: gtk::Entry,
    rows: RefCell<Vec<adw::ActionRow>>,
}

impl ListEditor {
    /// Builds the list and the add row, following the config.
    pub fn new(state: &Rc<State>, field: &'static Field) -> Rc<Self> {
        let list = gtk::ListBox::new();
        list.add_css_class("boxed-list");
        list.set_selection_mode(gtk::SelectionMode::None);
        list.set_margin_bottom(12);
        let add_title = format!("Add to {}", field.title.to_lowercase());
        let adder = widgets::row(&add_title, field.help);
        let entry: gtk::Entry = widgets::named(field.title);
        entry.set_placeholder_text(Some("kitty"));
        entry.set_valign(gtk::Align::Center);
        entry.set_hexpand(true);
        let add = widgets::button("Add", &add_title);
        add.add_css_class("suggested-action");
        adder.add_suffix(&entry);
        adder.add_suffix(&add);
        let editor = Rc::new(Self {
            state: Rc::clone(state),
            field,
            list,
            adder,
            entry,
            rows: RefCell::new(Vec::new()),
        });
        let submit = {
            let weak = Rc::downgrade(&editor);
            move || {
                if let Some(editor) = weak.upgrade() {
                    let text = editor.entry.text().to_string();
                    if editor.add(&text).is_ok() {
                        editor.entry.set_text("");
                    }
                }
            }
        };
        let on_click = submit.clone();
        add.connect_clicked(move |_| on_click());
        editor.entry.connect_activate(move |_| submit());
        let weak = Rc::downgrade(&editor);
        editor.entry.connect_changed(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.set_message(None);
            }
        });
        let weak = Rc::downgrade(&editor);
        state.connect_refresh(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.show();
            }
        });
        editor
    }

    /// The list of entries.
    pub fn list(&self) -> &gtk::ListBox {
        &self.list
    }

    /// The row with the entry and the Add button.
    pub fn adder(&self) -> &adw::ActionRow {
        &self.adder
    }

    /// The text entry.
    pub fn entry(&self) -> &gtk::Entry {
        &self.entry
    }

    /// The entries in effect.
    pub fn items(&self) -> Vec<String> {
        match self.state.value(self.field.key) {
            Value::List(items) => items,
            _ => Vec::new(),
        }
    }

    /// Adds `text` (trimmed) to the list and saves.
    ///
    /// # Errors
    /// A blank entry, a duplicate (ignoring case), or why it was not saved.
    pub fn add(self: &Rc<Self>, text: &str) -> Result<(), String> {
        let text = text.trim().to_owned();
        let mut items = self.items();
        let result = if text.is_empty() {
            Err("Type a name first".to_owned())
        } else if items.iter().any(|item| item.eq_ignore_ascii_case(&text)) {
            Err(format!("{text} is already in the list"))
        } else {
            items.push(text);
            self.state
                .set(self.field.key, Value::List(items))
                .map_err(|error| error.to_string())
        };
        self.set_message(result.as_ref().err().map(String::as_str));
        self.show();
        result
    }

    /// Removes the entry at `index` and saves.
    ///
    /// # Errors
    /// Why it was not saved.
    pub fn remove(self: &Rc<Self>, index: usize) -> Result<(), String> {
        let mut items = self.items();
        if index < items.len() {
            items.remove(index);
        }
        let result = self
            .state
            .set(self.field.key, Value::List(items))
            .map_err(|error| error.to_string());
        self.show();
        result
    }

    fn set_message(&self, error: Option<&str>) {
        widgets::set_subtitle(&self.adder, error.unwrap_or(self.field.help));
        widgets::set_error(&self.entry, error.is_some());
        widgets::set_subtitle_error(&self.adder, error.is_some());
    }

    /// Rebuilds the rows from the entries in effect.
    fn show(self: &Rc<Self>) {
        for row in self.rows.borrow_mut().drain(..) {
            self.list.remove(&row);
        }
        let items = self.items();
        if items.is_empty() {
            let row = widgets::row(&format!("No {}", self.field.title.to_lowercase()), "");
            row.add_css_class("dim-label");
            self.list.append(&row);
            self.rows.borrow_mut().push(row);
            return;
        }
        for (index, item) in items.iter().enumerate() {
            let row = widgets::row(item, "");
            let remove: gtk::Button = widgets::named(&format!("Remove {item}"));
            remove.set_icon_name("user-trash-symbolic");
            remove.set_valign(gtk::Align::Center);
            remove.set_tooltip_text(Some("Remove"));
            remove.add_css_class("flat");
            let weak = Rc::downgrade(self);
            remove.connect_clicked(move |_| {
                if let Some(editor) = weak.upgrade() {
                    // A failed save is already shown as a toast.
                    editor.remove(index).ok();
                }
            });
            row.add_suffix(&remove);
            self.list.append(&row);
            self.rows.borrow_mut().push(row);
        }
    }
}
