//! Small builders for the rows every page uses, each interactive widget with its accessible
//! name set to the row title (GTK 4.6 does not derive it from the row, and the UI test finds
//! widgets by name).

use adw::prelude::*;
use gtk::glib;

use crate::args::Page;

/// `text` escaped for an XML attribute or element.
fn xml_escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            c => escaped.push(c),
        }
    }
    escaped
}

/// A new `W` whose accessible name is `label`.
///
/// The Rust bindings expose `gtk_accessible_update_property` only for GTK 4.10, so the name is
/// set the way GTK 4.0 already supports: a `GtkBuilder` `<accessibility>` element.
pub fn named<W: IsA<gtk::Widget> + IsA<glib::Object> + glib::object::IsClass>(label: &str) -> W {
    let ui = format!(
        "<interface><object class=\"{}\" id=\"widget\"><accessibility>\
         <property name=\"label\">{}</property></accessibility></object></interface>",
        W::static_type().name(),
        xml_escape(label)
    );
    let builder = gtk::Builder::new();
    builder
        .add_from_string(&ui)
        .ok()
        .and_then(|()| builder.object::<W>("widget"))
        .unwrap_or_else(glib::Object::new::<W>)
}

/// A page for `page`.
pub fn page(page: Page) -> adw::PreferencesPage {
    adw::PreferencesPage::builder()
        .title(page.title())
        .icon_name(page.icon())
        .name(page.id())
        .build()
}

/// A group of rows; `description` may be empty.
pub fn group(title: &str, description: &str) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder().title(title).build();
    if !description.is_empty() {
        group.set_description(Some(description));
    }
    group
}

/// Row titles and subtitles are Pango markup in libadwaita 1.1: text from a file or an error
/// message must be escaped.
pub fn markup(text: &str) -> String {
    xml_escape(text)
}

/// A plain row; `subtitle` may be empty.
pub fn row(title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(markup(title)).build();
    set_subtitle(&row, subtitle);
    row.set_subtitle_lines(0);
    row
}

/// Sets a row's subtitle to plain `text` (empty hides it).
pub fn set_subtitle(row: &impl IsA<adw::ActionRow>, text: &str) {
    row.as_ref().set_subtitle(&markup(text));
}

/// A row with a switch at its end; activating the row toggles it.
pub fn switch_row(title: &str, subtitle: &str) -> (adw::ActionRow, gtk::Switch) {
    let row = row(title, subtitle);
    let switch: gtk::Switch = named(title);
    switch.set_valign(gtk::Align::Center);
    row.add_suffix(&switch);
    row.set_activatable_widget(Some(&switch));
    (row, switch)
}

/// A row with a whole-number spin button from `min` to `max`.
pub fn spin_row(
    title: &str,
    subtitle: &str,
    min: u32,
    max: u32,
) -> (adw::ActionRow, gtk::SpinButton) {
    let row = row(title, subtitle);
    let spin: gtk::SpinButton = named(title);
    spin.set_adjustment(&gtk::Adjustment::new(
        f64::from(min),
        f64::from(min),
        f64::from(max),
        1.0,
        10.0,
        0.0,
    ));
    spin.set_valign(gtk::Align::Center);
    spin.set_numeric(true);
    row.add_suffix(&spin);
    row.set_activatable_widget(Some(&spin));
    (row, spin)
}

/// The spin button's value as a `u32` (it never goes below its minimum of at least 1).
pub fn spin_value(spin: &gtk::SpinButton) -> u32 {
    u32::try_from(spin.value_as_int()).unwrap_or(0)
}

/// A row choosing one of `items`.
pub fn combo_row(title: &str, subtitle: &str, items: &[&str]) -> adw::ComboRow {
    let row: adw::ComboRow = named(title);
    row.set_title(&markup(title));
    row.set_model(Some(&gtk::StringList::new(items)));
    row.set_factory(Some(&choice_factory()));
    set_subtitle(&row, subtitle);
    row.set_subtitle_lines(0);
    row
}

/// Shows each choice in full: the stock factory ellipsizes the selected choice after a few
/// characters, which hides most of a descriptive choice title.
fn choice_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            let label = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::None)
                .build();
            item.set_child(Some(&label));
        }
    });
    factory.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let text = item
            .item()
            .and_downcast::<gtk::StringObject>()
            .map(|choice| choice.string());
        if let (Some(label), Some(text)) = (item.child().and_downcast::<gtk::Label>(), text) {
            label.set_text(&text);
        }
    });
    factory
}

/// A button for a row's end, vertically centred, named `title`.
pub fn button(label: &str, title: &str) -> gtk::Button {
    let button: gtk::Button = named(title);
    button.set_label(label);
    button.set_valign(gtk::Align::Center);
    button
}

/// Marks `widget` as wrong (red) or right.
pub fn set_error(widget: &impl IsA<gtk::Widget>, error: bool) {
    if error {
        widget.add_css_class("error");
    } else {
        widget.remove_css_class("error");
    }
}

/// Shows a row's subtitle in red (the `error-subtitle` class from the window's style sheet).
pub fn set_subtitle_error(row: &impl IsA<gtk::Widget>, error: bool) {
    if error {
        row.add_css_class("error-subtitle");
    } else {
        row.remove_css_class("error-subtitle");
    }
}

/// The position of `value` in `all`, for a combo row.
pub fn index_of<T: PartialEq>(all: &[T], value: &T) -> u32 {
    all.iter()
        .position(|item| item == value)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(0)
}

/// The item of `all` a combo row selected.
pub fn selected<T: Copy>(all: &[T], row: &adw::ComboRow) -> Option<T> {
    all.get(usize::try_from(row.selected()).ok()?).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_of_finds_or_defaults_to_the_first() {
        assert_eq!(index_of(&["a", "b", "c"], &"c"), 2);
        assert_eq!(index_of(&["a", "b"], &"z"), 0);
    }

    #[test]
    fn names_are_escaped_for_the_builder() {
        assert_eq!(
            xml_escape("Remove <a & 'b'>\""),
            "Remove &lt;a &amp; &apos;b&apos;&gt;&quot;"
        );
    }
}
