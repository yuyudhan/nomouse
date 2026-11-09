//! The shortcut recorder: a push button that, once clicked and told to record, takes the next
//! key press (with its modifiers, Command included) instead of letting it act.

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBezelStyle, NSButton, NSControl, NSEvent, NSEventType, NSResponder, NSView};
use objc2_foundation::{NSObject, NSRect};

use super::{UiEvent, modifiers, push, recording};

define_class!(
    // SAFETY: NSButton may be subclassed; `Recorder` overrides only responder methods with
    // their documented signatures, keeps no state of its own and does not implement Drop.
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    pub(crate) struct Recorder;

    impl Recorder {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if !self.capture(event) {
                // SAFETY: forwarding the same event to NSButton's own implementation.
                let () = unsafe { msg_send![super(self), keyDown: event] };
            }
        }

        // Command and Control chords reach views as key equivalents before `keyDown:`.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            if self.capture(event) {
                true
            } else {
                // SAFETY: forwarding the same event to NSButton's own implementation.
                unsafe { msg_send![super(self), performKeyEquivalent: event] }
            }
        }
    }
);

impl Recorder {
    /// A recorder button reporting `tag`.
    pub(crate) fn new(tag: isize, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSButton's designated initializer on a freshly allocated object.
        let button: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        button.setBezelStyle(NSBezelStyle::Push);
        button.setTag(tag);
        button
    }

    /// Queues the key press when this recorder is recording; whether it did.
    fn capture(&self, event: &NSEvent) -> bool {
        let tag = self.tag();
        if recording() != Some(tag) || event.r#type() != NSEventType::KeyDown {
            return false;
        }
        push(UiEvent::Key {
            tag,
            keycode: event.keyCode(),
            characters: event
                .charactersIgnoringModifiers()
                .map(|text| text.to_string())
                .unwrap_or_default(),
            modifiers: modifiers(event.modifierFlags()),
        });
        true
    }
}
