//! The Appearance page's extra: a live preview drawn by the overlay's own label renderer,
//! redrawn on every change and when the system switches between light and dark.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};
use nomouse_core::config::OverlayConfig;

use crate::controls::system_appearance;
use crate::state::State;
use crate::widgets;

/// The preview.
pub struct Preview {
    /// The group holding the card.
    pub group: adw::PreferencesGroup,
    /// The rendered labels.
    pub picture: gtk::Picture,
    error: gtk::Label,
}

impl Preview {
    /// Builds the card and follows the config and the system appearance.
    pub fn new(state: &Rc<State>) -> Rc<Self> {
        let group = widgets::group(
            "Preview",
            "Drawn by the overlay itself: a label, one with a letter typed, the highlighted one, another.",
        );
        let picture = gtk::Picture::new();
        picture.set_can_shrink(false);
        picture.set_halign(gtk::Align::Center);
        picture.set_valign(gtk::Align::Center);
        picture.set_alternative_text(Some("Label preview"));
        let error = gtk::Label::builder().wrap(true).visible(false).build();
        error.add_css_class("dim-label");
        let inner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .vexpand(true)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(24)
            .margin_end(24)
            .build();
        inner.append(&picture);
        inner.append(&error);
        let card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .height_request(96)
            .build();
        card.add_css_class("card");
        card.add_css_class("preview-card");
        card.append(&inner);
        group.add(&card);
        let preview = Rc::new(Self {
            group,
            picture,
            error,
        });
        let weak = Rc::downgrade(&preview);
        state.connect_changed(move |config| {
            if let Some(preview) = weak.upgrade() {
                preview.render(&config.overlay);
            }
        });
        let weak = Rc::downgrade(&preview);
        let follow = Rc::clone(state);
        adw::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(preview) = weak.upgrade() {
                preview.render(&follow.config().overlay);
            }
        });
        preview
    }

    /// Draws the sample labels for `overlay` under the current system appearance.
    pub fn render(&self, overlay: &OverlayConfig) {
        match nomouse_linux::preview::render(overlay, system_appearance()) {
            Ok(image) if image.width > 0 && image.height > 0 => {
                let (Ok(width), Ok(height)) =
                    (i32::try_from(image.width), i32::try_from(image.height))
                else {
                    return;
                };
                let stride = image.width as usize * 4;
                let texture = gdk::MemoryTexture::new(
                    width,
                    height,
                    gdk::MemoryFormat::B8g8r8a8Premultiplied,
                    &glib::Bytes::from_owned(image.pixels),
                    stride,
                );
                self.picture.set_paintable(Some(&texture));
                self.picture.set_visible(true);
                self.error.set_visible(false);
            }
            Ok(_) => self.show_error("there are no labels to draw"),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    fn show_error(&self, error: &str) {
        self.picture.set_visible(false);
        self.error
            .set_text(&format!("The preview cannot be drawn: {error}"));
        self.error.set_visible(true);
    }
}
