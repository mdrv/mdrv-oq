//! mdrv-oq: a tray app that watches the Z.AI coding-plan quota. The
//! window is the dashboard (hidden at start; opened from the tray), the
//! tray icon is the at-a-glance surface: the used percent drawn into a
//! colored tile, the full detail on hover.

use std::sync::OnceLock;
use std::time::Duration;

use mdrv_oq_core::model::QuotaState;
use mdrv_oq_core::{
    api, key, render, thresholds,
    settings::{self, Settings},
};
use nsh_base::theme;
use nsh_core::desktop::{TrayTitleRun, WindowAttributes};
use nsh_core::element::desktop::{current_window, quit, set_tray, tray, MenuItem};
use nsh_core::element::input;
use nsh_core::element::ImageSrc;
use nsh_core::notifications::Notification;
use nsh_core::platform::HttpRequest;
use nsh_core::prelude::*;
use nsh_core::time::unix_ms;

/// The settings snapshot the UI and the poller read.
#[derive(Debug, Clone, PartialEq)]
struct Cfg {
    interval_secs: i64,
    threshold_pct: f64,
    show_reset: bool,
    /// The key saved in the app, if any. Rendered only as its presence —
    /// never the value.
    settings_key: Option<String>,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg {
            interval_secs: settings::DEFAULT_INTERVAL_SECS,
            threshold_pct: settings::DEFAULT_THRESHOLD_PCT,
            show_reset: settings::DEFAULT_SHOW_RESET,
            settings_key: None,
        }
    }
}

/// The settings store, opened once for the app's lifetime.
static SETTINGS: OnceLock<Settings> = OnceLock::new();

fn load_cfg() -> Cfg {
    match SETTINGS.get() {
        Some(s) => Cfg {
            interval_secs: settings::interval_secs(s),
            threshold_pct: settings::threshold_pct(s),
            show_reset: settings::show_reset(s),
            settings_key: s.get(settings::API_KEY),
        },
        None => Cfg::default(),
    }
}

/// Prepares the window: it must not quit the app on close (the tray keeps
/// running), and the ✕ is disabled in favor of the header's Hide button.
fn prepare_window() {
    let w = current_window();
    let mut attrs = WindowAttributes::default();
    attrs.close_on_press = Some(false);
    attrs.closable = Some(false);
    w.set(attrs);
    w.hide();
}

/// One fetch + one tray update. The poll loop and the Refresh item share
/// it.
async fn poll_once(state: Signal<QuotaState>, cfg: Signal<Cfg>, busy: Signal<bool>) {
    let c = cfg.get();
    busy.set(true);
    let now = unix_ms() as i64;

    let lookup = key::KeyLookup {
        settings_key: c.settings_key.clone(),
        env_key: std::env::var("ZAI_API_KEY").ok(),
        opencode_db: None,
        auth_json: None,
    };
    let outcome = match key::resolve_key(&lookup) {
        Ok(ks) => {
            let request =
                HttpRequest::get(api::DEFAULT_API_URL).header("Authorization", &ks.key);
            match fetch(request).await {
                Ok(response) => {
                    let text = response.text();
                    match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(body) => api::parse_quota(&body),
                        Err(_) => Err("quota response is not JSON".to_string()),
                    }
                }
                Err(e) => Err(format!("HTTP: {e}")),
            }
        }
        Err(e) => Err(e),
    };

    let prev = state.get();
    let mut next = prev.clone();
    match outcome {
        Ok(sample) => next.apply_sample(sample, now),
        Err(err) => next.apply_error(err, now),
    }

    if thresholds::threshold_crossed(prev.percentage, prev.next_reset_ms, &next, c.threshold_pct) {
        fire(&next, &c, now, "Quota threshold reached");
    }
    if thresholds::error_onset(prev.error.as_deref(), next.error.as_deref()) {
        fire(&next, &c, now, "Quota check failed");
    }

    state.set(next.clone());
    busy.set(false);
    apply_tray(&state, &cfg, &busy);
}

/// An OS notification, silent, once per crossing.
fn fire(state: &QuotaState, cfg: &Cfg, now: i64, title: &str) {
    let body = match state.percentage {
        Some(p) => {
            let mut b = format!("{} used", render::pct_text(p));
            if cfg.show_reset {
                if let Some(reset) = state.next_reset_ms {
                    b.push_str(&format!(" \u{00b7} resets {}", render::reset_clock(reset)));
                }
            }
            b
        }
        None => state.error.clone().unwrap_or_default(),
    };
    let mut notification = Notification::new(
        format!("mdrv-oq-{now}"),
        title,
        body,
    );
    notification.silent = true;
    spawn(async move {
        let _ = show_notification(notification).await;
    });
}

/// The tray icon + title + tooltip + menu, re-set after every change
/// worth showing. The tile icon carries the percent everywhere; the title
/// run adds it as text on panels that show SNI labels; a primary click
/// opens the window (the menu is the secondary click).
fn apply_tray(state: &Signal<QuotaState>, cfg: &Signal<Cfg>, busy: &Signal<bool>) {
    let snapshot = state.get();
    let c = cfg.get();
    let now = unix_ms() as i64;
    let icon = match snapshot.percentage {
        Some(p) => render::icon_svg(p, c.threshold_pct),
        None => render::icon_svg_placeholder(),
    };
    let tip = render::tooltip(&snapshot, "mdrv-oq", c.show_reset, c.interval_secs, now);

    let (pct_run, pct_color) = match snapshot.percentage {
        Some(p) => (
            render::pct_text(p),
            Some(title_color(p, c.threshold_pct)),
        ),
        None => ("\u{2026}".to_string(), None),
    };
    let run = TrayTitleRun::new(pct_run);
    let run = match pct_color {
        Some(color) => run.color(color),
        None => run,
    };

    let menu: Vec<MenuItem> = vec![
        MenuItem::new("Open").on_select(|| {
            let w = current_window();
            w.show();
            w.focus();
        }),
        {
            let state = state.clone();
            let cfg = cfg.clone();
            let busy = busy.clone();
            MenuItem::new("Refresh now").on_select(move || {
                let state = state.clone();
                let cfg = cfg.clone();
                let busy = busy.clone();
                spawn(async move {
                    poll_once(state, cfg, busy).await;
                });
            })
        },
        MenuItem::separator(),
        MenuItem::new("Quit").on_select(quit),
    ];

    set_tray(
        tray()
            .icon(icon_source(&icon))
            .tooltip(tip)
            .title([run])
            .menu(menu)
            .on_click(|_| {
                let w = current_window();
                w.show();
                w.focus();
            }),
    );
}

/// The title run's color, readable on a panel's background.
fn title_color(pct: f64, threshold: f64) -> Hsla {
    let (h, s, l) = hsl_of(match render::icon_color(pct, threshold) {
        render::GREEN => "#16a34a",
        render::AMBER => "#d97706",
        _ => "#dc2626",
    });
    hsla(h, s, l.clamp(0.25, 0.55), 1.0)
}

/// The tray icon: an SVG `data:` URI, decoded by the image pipeline.
fn icon_source(svg: &str) -> ImageSrc {
    ImageSrc::from(render::svg_data_uri(svg))
}

/// `app settings` / `ZAI_API_KEY` / `OpenCode credential DB` / `none` —
/// where the next fetch's key would come from. Presence only; the key's
/// value is never rendered.
fn key_origin_line(cfg: &Cfg) -> String {
    if cfg.settings_key.as_deref().is_some_and(|k| !k.trim().is_empty()) {
        return "key: app settings".into();
    }
    if std::env::var("ZAI_API_KEY")
        .ok()
        .is_some_and(|k| !k.trim().is_empty())
    {
        return "key: ZAI_API_KEY".into();
    }
    match key::opencode_db_path() {
        Some(p) if p.exists() => "key: OpenCode credential DB".into(),
        _ => match key::auth_json_path() {
            Some(p) if p.exists() => "key: OpenCode auth.json".into(),
            _ => "key: none found".into(),
        },
    }
}

fn interval_text(secs: i64) -> String {
    if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// The big number on the dashboard, or its stand-in.
fn big_text(state: &QuotaState) -> String {
    match state.percentage {
        Some(p) => render::pct_text(p),
        None if state.error.is_some() => "\u{2716}".into(),
        None => "\u{2026}".into(),
    }
}

/// The percent's color for the window, readable on the theme.
fn pct_color(state: &QuotaState, cfg: &Cfg, fallback: Hsla) -> Hsla {
    let Some(p) = state.percentage else {
        return fallback;
    };
    let (h, s, l) = hsl_of(match render::icon_color(p, cfg.threshold_pct) {
        render::GREEN => "#16a34a",
        render::AMBER => "#d97706",
        _ => "#dc2626",
    });
    hsla(h, s, l.clamp(0.25, 0.55), fallback.a)
}

/// `#rrggbb` → (hue 0..1, saturation, lightness).
fn hsl_of(hex: &str) -> (f32, f32, f32) {
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
    let (r, g, b) = (ch(1), ch(3), ch(5));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d == 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    ((if h < 0.0 { h + 360.0 } else { h }) / 360.0, s, l)
}

/// A label / value line.
fn row(label: &'static str, value: impl IntoReactive<nsh_core::element::SharedString>) -> impl IntoElement {
    view()
        .flex_row()
        .w_full()
        .justify_between()
        .child(
            text(label)
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(text(value).text_size(px(13.0)))
}

/// A small pressable chip with our inline styling.
fn chip(label: impl IntoReactive<nsh_core::element::SharedString>, on_press: impl Fn() + 'static) -> impl IntoElement {
    view()
        .px(px(10.0))
        .py(px(4.0))
        .rounded(px(6.0))
        .bg(move || theme().secondary)
        .text_color(move || theme().secondary_foreground)
        .on_press(on_press)
        .child(text(label).text_size(px(13.0)))
}

/// A settings line: its name and current value stacked on the left, the
/// controls on the right.
fn setting_row(
    name: &'static str,
    value: impl IntoReactive<nsh_core::element::SharedString>,
    controls: impl IntoElement,
) -> impl IntoElement {
    view()
        .flex_row()
        .w_full()
        .items_center()
        .justify_between()
        .child(
            view()
                .flex_col()
                .gap(px(2.0))
                .child(
                    text(name)
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(text(value).text_size(px(13.0))),
        )
        .child(controls)
}

#[component]
pub fn App() -> impl IntoElement {
    let state = signal(QuotaState::fresh());
    let cfg = signal(Cfg::default());
    let busy = signal(false);
    let key_draft = signal(String::new());

    // Store, tray, hide, first poll, then the loop.
    {
        let state = state.clone();
        let cfg = cfg.clone();
        let busy = busy.clone();
        spawn(async move {
            if SETTINGS.get().is_none() {
                match settings::Settings::open(&settings::default_db_path()) {
                    Ok(s) => {
                        let _ = SETTINGS.set(s);
                    }
                    Err(e) => {
                        let mut st = state.get();
                        st.apply_error(format!("settings: {e}"), unix_ms() as i64);
                        state.set(st);
                    }
                }
            }
            cfg.set(load_cfg());
            prepare_window();
            apply_tray(&state, &cfg, &busy);
            loop {
                poll_once(state.clone(), cfg.clone(), busy.clone()).await;
                let interval = cfg.get().interval_secs.clamp(30, 86_400) as u64;
                timer(Duration::from_secs(interval)).await;
            }
        });
    }

    // Clones for the reactive closures below.
    let big_state = state.clone();
    let big_cfg = cfg.clone();
    let bar_state = state.clone();
    let bar_cfg = cfg.clone();
    let origin_cfg = cfg.clone();
    let resets_state = state.clone();
    let resets_cfg = cfg.clone();
    let level_state = state.clone();
    let checked_state = state.clone();
    let status_state = state.clone();
    let status_cfg = cfg.clone();
    let tray_state = state.clone();
    let tray_cfg = cfg.clone();
    let tray_busy = busy.clone();
    let threshold_cfg = cfg.clone();
    let threshold_tray_state = state.clone();
    let threshold_tray_busy = busy.clone();
    let reset_cfg = cfg.clone();
    let reset_tray_state = state.clone();
    let reset_tray_busy = busy.clone();
    let save_key_draft = key_draft.clone();
    let save_cfg = cfg.clone();
    let clear_cfg = cfg.clone();
    let interval_cfg = cfg.clone();

    view()
        .flex_1()
        .flex_col()
        .p_safe_area()
        .p(px(20.0))
        .gap(px(12.0))
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        // Header: identity + where the key comes from + Hide.
        .child(
            view()
                .flex_row()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    view()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            text("GLM \u{00b7} Z.AI Coding Plan")
                                .text_size(px(15.0))
                                .text_color(move || theme().foreground),
                        )
                        .child(
                            text(move || key_origin_line(&origin_cfg.get()))
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(
                    chip("Hide", || {
                        current_window().hide();
                    }),
                ),
        )
        // The number.
        .child(
            view()
                .flex_col()
                .w_full()
                .items_center()
                .gap(px(6.0))
                .p(px(10.0))
                .child(
                    text(move || big_text(&big_state.get()))
                        .text_size(px(64.0))
                        .text_color(move || {
                            pct_color(&big_state.get(), &big_cfg.get(), hex_bg("#6b7280"))
                        }),
                )
                .child(
                    text(move || {
                        let s = status_state.get();
                        let c = status_cfg.get();
                        match &s.error {
                            Some(e) => e.clone(),
                            None if s.percentage.is_none() => {
                                "waiting for the first check".into()
                            }
                            None if s.is_stale(c.interval_secs, unix_ms() as i64) => {
                                "stale \u{2014} last check is old".into()
                            }
                            None => "ok".into(),
                        }
                    })
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
                ),
        )
        // The bar.
        .child(
            view()
                .flex_row()
                .w_full()
                .h(px(8.0))
                .rounded(px(4.0))
                .bg(move || theme().secondary)
                .overflow_hidden()
                .child(
                    view()
                        .h_full()
                        .rounded(px(4.0))
                        .bg(move || {
                            let c = bar_cfg.get();
                            match bar_state.get().percentage {
                                Some(p) => match render::icon_color(p, c.threshold_pct) {
                                    render::GREEN => hex_bg("#22c55e"),
                                    render::AMBER => hex_bg("#f59e0b"),
                                    _ => hex_bg("#ef4444"),
                                },
                                None => hex_bg("#9ca3af"),
                            }
                        })
                        .w(move || {
                            let p = bar_state
                                .get()
                                .percentage
                                .unwrap_or(0.0)
                                .clamp(0.0, 100.0);
                            px((296.0 * p / 100.0) as f32)
                        }),
                ),
        )
        // Details.
        .child(row(
            "Resets",
            move || {
                let s = resets_state.get();
                let c = resets_cfg.get();
                match (c.show_reset, s.next_reset_ms) {
                    (true, Some(reset)) => format!(
                        "{} \u{00b7} {}",
                        render::reset_clock(reset),
                        render::until_reset(reset, unix_ms() as i64)
                    ),
                    (true, None) => "\u{2014}".into(),
                    (false, _) => "hidden".into(),
                }
            },
        ))
        .child(row(
            "Level",
            move || level_state.get().level.unwrap_or_else(|| "\u{2014}".into()),
        ))
        .child(row("Interval", {
            move || interval_text(interval_cfg.get().interval_secs)
        }))
        .child(row(
            "Checked",
            move || {
                let s = checked_state.get();
                if s.fetched_at_ms == 0 {
                    "never".into()
                } else {
                    format!("{} ago", render::age_text(s.fetched_at_ms, unix_ms() as i64))
                }
            },
        ))
        // Settings.
        .child(
            view()
                .flex_col()
                .w_full()
                .gap(px(10.0))
                .p(px(12.0))
                .rounded(px(10.0))
                .bg(move || theme().card)
                .child(
                    text("Settings")
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    setting_row(
                        "Alert threshold",
                        move || render::pct_text(threshold_cfg.get().threshold_pct),
                        view()
                            .flex_row()
                            .gap(px(6.0))
                            .child(
                                chip("\u{2212}", {
                                    let cfg = threshold_cfg.clone();
                                    move || bump_threshold(&cfg, -5.0)
                                }),
                            )
                            .child(
                                chip("+", {
                                    let cfg = threshold_cfg.clone();
                                    move || bump_threshold(&cfg, 5.0)
                                }),
                            ),
                    ),
                )
                .child(setting_row(
                    "Reset time",
                    move || {
                        if reset_cfg.get().show_reset {
                            "shown".to_string()
                        } else {
                            "hidden".to_string()
                        }
                    },
                    chip("toggle", {
                        let cfg = reset_cfg.clone();
                        let tray_state = tray_state.clone();
                        let tray_busy = tray_busy.clone();
                        move || {
                            let mut c = cfg.get();
                            c.show_reset = !c.show_reset;
                            if let Some(s) = SETTINGS.get() {
                                let _ = s.set(
                                    settings::SHOW_RESET,
                                    Some(if c.show_reset { "1" } else { "0" }),
                                );
                            }
                            cfg.set(c);
                            apply_tray(&tray_state, &cfg, &tray_busy);
                        }
                    }),
                ))
                .child(
                    view()
                        .flex_row()
                        .w_full()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            input()
                                .flex_1()
                                .value(key_draft.clone())
                                .placeholder("API key (overrides env / OpenCode)")
                                .masked(true)
                                .on_change(move |v: &str| key_draft.set(v.to_string())),
                        )
                        .child(
                            chip("Save", {
                                let key_draft = save_key_draft.clone();
                                let cfg = save_cfg.clone();
                                move || {
                                    let mut v = key_draft.get();
                                    mdrv_oq_core::model::one_line(&mut v, 200);
                                    let v = v.trim().to_string();
                                    if let Some(s) = SETTINGS.get() {
                                        let _ = s.set(
                                            settings::API_KEY,
                                            if v.is_empty() { None } else { Some(&v) },
                                        );
                                    }
                                    key_draft.set(String::new());
                                    let c = load_cfg();
                                    cfg.set(c);
                                }
                            }),
                        )
                        .child(
                            chip("Clear", {
                                let cfg = clear_cfg.clone();
                                move || {
                                    if let Some(s) = SETTINGS.get() {
                                        let _ = s.set(settings::API_KEY, None);
                                    }
                                    let c = load_cfg();
                                    cfg.set(c);
                                }
                            }),
                        ),
                )
                .child(
                    text("Empty key = ZAI_API_KEY, or OpenCode's stored credentials")
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            text("Closing this window keeps the tray running. Quit from the tray menu.")
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
        )
}

/// A bar/tile background color straight from the tray hexes.
fn hex_bg(hex: &str) -> Hsla {
    let (h, s, l) = hsl_of(hex);
    hsla(h, s, l, 1.0)
}

fn bump_threshold(cfg: &Signal<Cfg>, delta: f64) {
    let mut c = cfg.get();
    c.threshold_pct = (c.threshold_pct + delta).clamp(50.0, 99.0);
    if let Some(s) = SETTINGS.get() {
        let _ = s.set(settings::THRESHOLD_PCT, Some(&format!("{}", c.threshold_pct)));
    }
    cfg.set(c);
}
