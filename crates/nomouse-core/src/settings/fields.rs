//! What a settings window shows: every configuration key with its section, title, help text
//! and kind of control, so the macOS and Linux windows list the same settings in the same
//! order and a test can check that no key is missing.

/// A group of settings: one tab or page of a settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Section {
    /// The global shortcuts of each mode.
    Hotkeys,
    /// The label letters, what picking one does and which keys act.
    Labels,
    /// Label size, opacity, theme and colours.
    Appearance,
    /// Scroll mode's steps.
    Scroll,
    /// Search and how much of a web page a scan reads.
    Search,
    /// Apps where activation shows no labels.
    IgnoredApps,
    /// Everything else: accessibility actions, per-app switches, the file itself.
    Advanced,
}

impl Section {
    /// Every section, in the order a window shows them.
    pub const ALL: [Self; 7] = [
        Self::Hotkeys,
        Self::Labels,
        Self::Appearance,
        Self::Scroll,
        Self::Search,
        Self::IgnoredApps,
        Self::Advanced,
    ];

    /// The section's title.
    pub const fn title(self) -> &'static str {
        match self {
            Self::Hotkeys => "Hotkeys",
            Self::Labels => "Labels",
            Self::Appearance => "Appearance",
            Self::Scroll => "Scroll",
            Self::Search => "Search",
            Self::IgnoredApps => "Ignored apps",
            Self::Advanced => "Advanced",
        }
    }

    /// One line saying what the section is for.
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Hotkeys => "The shortcuts that start each mode, from any app.",
            Self::Labels => "The letters labels are made of and what picking a label does.",
            Self::Appearance => "How labels look.",
            Self::Scroll => "How far scroll mode moves per key press.",
            Self::Search => "Search finds elements by their text; browsers can be read whole.",
            Self::IgnoredApps => "Apps where the hotkeys show no labels.",
            Self::Advanced => "Accessibility actions, per-app switches and the config file.",
        }
    }

    /// The fields of this section, in order.
    pub fn fields(self) -> impl Iterator<Item = &'static Field> {
        FIELDS.iter().filter(move |field| field.section == self)
    }
}

/// One allowed value of a [`Kind::Choice`] field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    /// The value written to the file.
    pub value: &'static str,
    /// What a window shows for it.
    pub title: &'static str,
}

/// The kind of control a field needs, with its limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Free text, validated when set.
    Text,
    /// One of a fixed set of values.
    Choice(&'static [Choice]),
    /// A whole number between `min` and `max`, both included.
    Number {
        /// Smallest allowed.
        min: u32,
        /// Largest allowed.
        max: u32,
    },
    /// On or off.
    Toggle,
    /// A global shortcut such as `ctrl+space`.
    Hotkey,
    /// An optional `#rrggbb` colour; unset means the theme's.
    Colour,
    /// A list of application ids.
    Apps,
}

/// One configuration key as a settings window shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    /// The `section.key` path in `config.toml`.
    pub key: &'static str,
    /// Where the field is shown.
    pub section: Section,
    /// Its label.
    pub title: &'static str,
    /// A sentence of help below or beside it.
    pub help: &'static str,
    /// The control.
    pub kind: Kind,
    /// The only OS (`std::env::consts::OS`) where the setting does anything, if it is
    /// limited to one.
    pub only_on: Option<&'static str>,
}

impl Field {
    /// Whether the field applies on the OS named like `std::env::consts::OS`.
    pub fn applies_on(&self, os: &str) -> bool {
        self.only_on.is_none_or(|only| only == os)
    }
}

const SCOPES: &[Choice] = &[
    Choice {
        value: "focused-window",
        title: "The focused window",
    },
    Choice {
        value: "screen",
        title: "Every window on every screen",
    },
];

const ACTIONS: &[Choice] = &[
    Choice {
        value: "click",
        title: "Click",
    },
    Choice {
        value: "right-click",
        title: "Right-click",
    },
    Choice {
        value: "double-click",
        title: "Double-click",
    },
    Choice {
        value: "hover",
        title: "Move the pointer there",
    },
    Choice {
        value: "drag",
        title: "Drag (pick the start, then the end)",
    },
    Choice {
        value: "scroll",
        title: "Scroll (pick an area, then scroll it)",
    },
];

const KEYS: &[Choice] = &[
    Choice {
        value: "nomouse",
        title: "A complete label acts at once",
    },
    Choice {
        value: "homerow",
        title: "Homerow: a label highlights, Return acts",
    },
];

const BROWSER: &[Choice] = &[
    Choice {
        value: "fast",
        title: "Fast: only what is in view",
    },
    Choice {
        value: "exhaustive",
        title: "Exhaustive: the whole page",
    },
];

const THEMES: &[Choice] = &[
    Choice {
        value: "original",
        title: "Original (yellow)",
    },
    Choice {
        value: "light",
        title: "Light",
    },
    Choice {
        value: "dark",
        title: "Dark",
    },
    Choice {
        value: "auto",
        title: "Auto (follow the system)",
    },
];

const fn field(key: &'static str, section: Section, title: &'static str, kind: Kind) -> Field {
    Field {
        key,
        section,
        title,
        help: "",
        kind,
        only_on: None,
    }
}

const fn helped(mut field: Field, help: &'static str) -> Field {
    field.help = help;
    field
}

/// Every configuration key, grouped by section in window order.
pub const FIELDS: &[Field] = &[
    helped(
        field("hotkeys.hints", Section::Hotkeys, "Hints", Kind::Hotkey),
        "Label the window; type a label to click.",
    ),
    helped(
        field(
            "hotkeys.continuous",
            Section::Hotkeys,
            "Continuous hints",
            Kind::Hotkey,
        ),
        "Label again after every click, until Escape.",
    ),
    helped(
        field("hotkeys.scroll", Section::Hotkeys, "Scroll", Kind::Hotkey),
        "Pick a scrollable area, then scroll it with h j k l.",
    ),
    helped(
        field("hotkeys.search", Section::Hotkeys, "Search", Kind::Hotkey),
        "Type part of an element's text; Return clicks the best match.",
    ),
    helped(
        field(
            "hints.alphabet",
            Section::Labels,
            "Label letters",
            Kind::Text,
        ),
        "Most preferred first; at least two letters, without i, l and o.",
    ),
    helped(
        field(
            "hints.keys",
            Section::Labels,
            "Label keys",
            Kind::Choice(KEYS),
        ),
        "Homerow: Return clicks, Return twice double-clicks, Shift-Return right-clicks, \
         Cmd-Return cmd-clicks.",
    ),
    helped(
        field(
            "hints.action",
            Section::Labels,
            "Picking a label",
            Kind::Choice(ACTIONS),
        ),
        "What the hints hotkey does with the picked element.",
    ),
    helped(
        field(
            "hints.scope",
            Section::Labels,
            "Label",
            Kind::Choice(SCOPES),
        ),
        "Which windows get labels.",
    ),
    helped(
        field(
            "overlay.theme",
            Section::Appearance,
            "Theme",
            Kind::Choice(THEMES),
        ),
        "Auto follows the system's light or dark appearance.",
    ),
    field(
        "overlay.font_size",
        Section::Appearance,
        "Label size",
        Kind::Number { min: 6, max: 72 },
    ),
    helped(
        field(
            "overlay.opacity",
            Section::Appearance,
            "Opacity",
            Kind::Number { min: 10, max: 100 },
        ),
        "Percent.",
    ),
    helped(
        field(
            "overlay.background",
            Section::Appearance,
            "Background",
            Kind::Colour,
        ),
        "Unset: the theme's colour.",
    ),
    field(
        "overlay.foreground",
        Section::Appearance,
        "Text",
        Kind::Colour,
    ),
    field(
        "overlay.typed_foreground",
        Section::Appearance,
        "Typed letters",
        Kind::Colour,
    ),
    field(
        "overlay.border",
        Section::Appearance,
        "Border",
        Kind::Colour,
    ),
    helped(
        field(
            "scroll.step",
            Section::Scroll,
            "Step",
            Kind::Number { min: 1, max: 50 },
        ),
        "Wheel steps per h j k l press; d and u scroll five times this.",
    ),
    helped(
        field(
            "scroll.dash_step",
            Section::Scroll,
            "Shift step",
            Kind::Number { min: 1, max: 100 },
        ),
        "Wheel steps per Shift with H J K L.",
    ),
    helped(
        field(
            "hints.browser",
            Section::Search,
            "Web pages",
            Kind::Choice(BROWSER),
        ),
        "Exhaustive lets search find links scrolled out of view, and scrolls to them.",
    ),
    helped(
        field(
            "ignore.apps",
            Section::IgnoredApps,
            "Ignored apps",
            Kind::Apps,
        ),
        "Bundle ids, executable names, Wayland app ids or X11 window classes.",
    ),
    helped(
        field(
            "hints.semantic_click",
            Section::Advanced,
            "Press through accessibility",
            Kind::Toggle,
        ),
        "Press elements through the accessibility API when they allow it, without moving \
         the pointer.",
    ),
    Field {
        only_on: Some("macos"),
        ..helped(
            field(
                "macos.enhanced_ui_apps",
                Section::Advanced,
                "Chromium and Electron apps",
                Kind::Apps,
            ),
            "Apps whose accessibility tree nomouse switches on (AXEnhancedUserInterface).",
        )
    },
];

/// The field for `key` (`section.key`), if the configuration has one.
pub fn field_for(key: &str) -> Option<&'static Field> {
    FIELDS.iter().find(|field| field.key == key)
}
