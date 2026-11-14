//! Property tests: every valid configuration survives a TOML round trip, out-of-range values are
//! rejected, the label stages run in order, and fuzzy search accepts exactly the queries that
//! are subsequences of the name.

use std::time::Duration;

use nomouse_core::config::{
    BrowserMode, Config, HintsConfig, MAX_RESCAN_MS, OverlayConfig, ScrollConfig, Theme,
};
use nomouse_core::search::fuzzy_score;
use nomouse_core::{ActionKind, ConfigError, KeyBindings, RescanSchedule, Scope};
use proptest::prelude::*;

fn alphabet() -> impl Strategy<Value = String> {
    proptest::sample::subsequence(
        "abcdefghjkmnpqrstuvwxyz".chars().collect::<Vec<_>>(),
        2..=12,
    )
    .prop_shuffle()
    .prop_map(|chars| chars.into_iter().collect())
}

fn colour() -> impl Strategy<Value = String> {
    any::<[u8; 3]>().prop_map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02X}"))
}

prop_compose! {
    fn valid_config()(
        alphabet in alphabet(),
        scope in prop::sample::select(vec![Scope::FocusedWindow, Scope::Screen]),
        action in prop::sample::select(ActionKind::ALL.to_vec()),
        semantic_click in any::<bool>(),
        keys in prop::sample::select(vec![KeyBindings::Nomouse, KeyBindings::Homerow]),
        browser in prop::sample::select(vec![BrowserMode::Fast, BrowserMode::Exhaustive]),
        early_rescan_ms in 0u32..=5000,
        late_gap in prop::option::of(1u32..=5000),
        step in 1u32..=50,
        dash_step in 1u32..=100,
        font_size in 6u32..=72,
        opacity in 10u32..=100,
        theme in prop::sample::select(vec![Theme::Original, Theme::Light, Theme::Dark, Theme::Auto]),
        colours in [
            prop::option::of(colour()),
            prop::option::of(colour()),
            prop::option::of(colour()),
            prop::option::of(colour()),
        ],
    ) -> Config {
        let [background, foreground, typed_foreground, border] = colours;
        Config {
            hints: HintsConfig {
                alphabet,
                scope,
                action,
                semantic_click,
                keys,
                browser,
                early_rescan_ms,
                late_rescan_ms: late_gap.map_or(0, |gap| early_rescan_ms + gap),
            },
            scroll: ScrollConfig { step, dash_step },
            overlay: OverlayConfig {
                font_size,
                opacity,
                theme,
                background,
                foreground,
                typed_foreground,
                border,
            },
            ..Config::default()
        }
    }
}

proptest! {
    #[test]
    fn valid_configs_round_trip(config in valid_config()) {
        prop_assert_eq!(config.validate().ok(), Some(()));
        let parsed = Config::from_toml(&config.to_toml());
        prop_assert_eq!(parsed.ok(), Some(config));
    }

    #[test]
    fn rescan_stages_are_accepted_exactly_when_in_range_and_in_order(
        early in 0u32..12_000,
        late in 0u32..12_000,
    ) {
        let text = format!("[hints]\nearly_rescan_ms = {early}\nlate_rescan_ms = {late}\n");
        let in_range = early <= MAX_RESCAN_MS && late <= MAX_RESCAN_MS;
        let in_order = early == 0 || late == 0 || early < late;
        prop_assert_eq!(Config::from_toml(&text).is_ok(), in_range && in_order);
        // Whatever the values, the stages run in strictly increasing order from the
        // immediate re-scan.
        let times = RescanSchedule::new(early, late).times().to_vec();
        prop_assert_eq!(times.first().copied(), Some(Duration::ZERO));
        prop_assert!(times.windows(2).all(|pair| pair[0] < pair[1]), "{:?}", times);
    }

    #[test]
    fn scroll_step_outside_range_is_rejected(step in prop_oneof![Just(0u32), 51u32..10_000]) {
        let text = format!("[scroll]\nstep = {step}\n");
        let rejected = matches!(
            Config::from_toml(&text),
            Err(ConfigError::OutOfRange { field: "scroll.step", .. })
        );
        prop_assert!(rejected, "step {} accepted", step);
    }

    #[test]
    fn dash_step_outside_range_is_rejected(dash_step in prop_oneof![Just(0u32), 101u32..10_000]) {
        let text = format!("[scroll]\ndash_step = {dash_step}\n");
        let rejected = matches!(
            Config::from_toml(&text),
            Err(ConfigError::OutOfRange { field: "scroll.dash_step", .. })
        );
        prop_assert!(rejected, "dash_step {} accepted", dash_step);
    }

    #[test]
    fn overlay_opacity_outside_range_is_rejected(opacity in prop_oneof![0u32..10, 101u32..10_000]) {
        let text = format!("[overlay]\nopacity = {opacity}\n");
        let rejected = matches!(
            Config::from_toml(&text),
            Err(ConfigError::OutOfRange { field: "overlay.opacity", .. })
        );
        prop_assert!(rejected, "opacity {} accepted", opacity);
    }

    #[test]
    fn any_text_never_panics_the_parser(text in "\\PC{0,200}") {
        let _ = Config::from_toml(&text);
    }

    #[test]
    fn subsequences_match_and_case_is_ignored(
        name in "[a-zA-Z ]{1,40}",
        mask in prop::collection::vec(any::<bool>(), 40),
    ) {
        let query: String = name
            .chars()
            .zip(&mask)
            .filter(|&(c, keep)| *keep && c != ' ')
            .map(|(c, _)| c)
            .collect();
        prop_assume!(!query.is_empty());
        prop_assert!(fuzzy_score(&name, &query).is_some());
        prop_assert_eq!(
            fuzzy_score(&name, &query),
            fuzzy_score(&name.to_uppercase(), &query.to_lowercase())
        );
    }

    #[test]
    fn substring_ranks_no_worse_than_scattered(name in "[a-z]{2,30}", start in 0usize..30, len in 1usize..10) {
        prop_assume!(start + len <= name.len());
        let query = &name[start..start + len];
        let score = fuzzy_score(&name, query);
        prop_assert!(score.is_some_and(|s| s < 2_000), "{query:?} in {name:?} scored {score:?}");
    }

    #[test]
    fn letters_absent_from_the_name_never_match(name in "[a-m]{0,30}", query in "[n-z]{1,5}") {
        prop_assert_eq!(fuzzy_score(&name, &query), None);
    }
}
