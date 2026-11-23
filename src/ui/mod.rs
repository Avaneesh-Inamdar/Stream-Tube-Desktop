pub mod app;
pub mod app_icon;
pub mod cards;
pub mod image_cache;
pub mod pages;
pub mod pip;
pub mod slot;
pub mod widgets;

use gpui_kit::*;
pub use gpui_kit::prelude::*;
use std::sync::Arc;

// ───────────────────────── palette ─────────────────────────

#[derive(Clone, Copy)]
pub struct Pal {
    pub dark: bool,
    pub bg: Hsla,
    pub surface: Hsla,
    pub surface2: Hsla,
    pub hover: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub text2: Hsla,
    pub text3: Hsla,
    pub accent: Hsla,
    pub link: Hsla,
    pub chip: Hsla,
    pub chip_active_bg: Hsla,
    pub chip_active_fg: Hsla,
    pub overlay: Hsla,
}

impl Global for Pal {}

impl Pal {
    pub fn new(dark: bool, accent: u32) -> Self {
        if dark {
            Self {
                dark,
                bg: rgb(0x0f0f0f).into(),
                surface: rgb(0x212121).into(),
                surface2: rgb(0x282828).into(),
                hover: rgb(0x3a3a3a).into(),
                border: rgb(0x303030).into(),
                text: rgb(0xf1f1f1).into(),
                text2: rgb(0xaaaaaa).into(),
                text3: rgb(0x717171).into(),
                accent: rgb(accent).into(),
                link: rgb(0x3ea6ff).into(),
                chip: rgb(0x272727).into(),
                chip_active_bg: rgb(0xf1f1f1).into(),
                chip_active_fg: rgb(0x0f0f0f).into(),
                overlay: hsla(0., 0., 0., 0.8),
            }
        } else {
            Self {
                dark,
                bg: rgb(0xffffff).into(),
                surface: rgb(0xf2f2f2).into(),
                surface2: rgb(0xffffff).into(),
                hover: rgb(0xe5e5e5).into(),
                border: rgb(0xe0e0e0).into(),
                text: rgb(0x0f0f0f).into(),
                text2: rgb(0x606060).into(),
                text3: rgb(0x909090).into(),
                accent: rgb(accent).into(),
                link: rgb(0x065fd4).into(),
                chip: rgb(0xf2f2f2).into(),
                chip_active_bg: rgb(0x0f0f0f).into(),
                chip_active_fg: rgb(0xffffff).into(),
                overlay: hsla(0., 0., 0., 0.8),
            }
        }
    }
}

pub fn pal(cx: &App) -> Pal {
    *cx.global::<Pal>()
}

pub fn apply_theme(window: Option<&mut Window>, cx: &mut App) {
    let s = crate::store::settings_snapshot();
    let dark = match s.theme.as_str() {
        "light" => false,
        "system" => window.as_ref().map(|w| matches!(w.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark)).unwrap_or(true),
        _ => true,
    };
    let p = Pal::new(dark, s.accent);
    cx.set_global(p);
    gpui_kit::component::Theme::change(
        if dark { gpui_kit::component::ThemeMode::Dark } else { gpui_kit::component::ThemeMode::Light },
        window,
        cx,
    );
    // text cursor, focus rings and selections follow the accent color
    let t = gpui_kit::component::Theme::global_mut(cx);
    t.caret = p.accent;
    t.ring = p.accent;
    t.selection = p.accent.opacity(0.35);
    cx.refresh_windows();
}

// ───────────────────────── icons ─────────────────────────

pub fn icon(name: &str) -> Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).flex_none().size_5()
}

// ───────────────────────── background work ─────────────────────────

/// Runs blocking work on a dedicated OS thread and returns a future for its result.
pub fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> impl std::future::Future<Output = T> {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.expect("worker thread panicked") }
}

// ───────────────────────── HTTP client for gpui img() ─────────────────────────

pub struct UreqHttp {
    ua: http_client::http::HeaderValue,
}

impl UreqHttp {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { ua: http_client::http::HeaderValue::from_static(crate::api::http::BROWSER_UA) })
    }
}

impl http_client::HttpClient for UreqHttp {
    fn user_agent(&self) -> Option<&http_client::http::HeaderValue> {
        Some(&self.ua)
    }

    fn proxy(&self) -> Option<&http_client::Url> {
        None
    }

    fn send(
        &self,
        req: http_client::Request<http_client::AsyncBody>,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<http_client::Response<http_client::AsyncBody>>> {
        let uri = req.uri().to_string();
        Box::pin(async move {
            let bytes = blocking(move || crate::api::http::get_bytes(&uri, &[])).await?;
            Ok(http_client::Response::builder()
                .status(200)
                .body(http_client::AsyncBody::from(bytes))?)
        })
    }
}

// ───────────────────────── toasts ─────────────────────────

#[derive(Clone)]
pub struct Toast {
    pub id: u64,
    pub text: SharedString,
    pub action: Option<(SharedString, Arc<dyn Fn(&mut Window, &mut App)>)>,
    /// further buttons after `action`
    pub extra: Vec<(SharedString, Arc<dyn Fn(&mut Window, &mut App)>)>,
}

#[derive(Default)]
pub struct Toasts {
    pub items: Vec<Toast>,
    pub next: u64,
}

impl Global for Toasts {}

/// Opens a web link in the browser. Links from comments, descriptions and release
/// notes are untrusted, so only http(s) goes to the system: never files, network
/// shares or other programs' URL schemes.
pub fn open_web(url: &str) {
    if is_web_url(url) {
        let _ = open::that(url.trim());
    }
}

pub fn is_web_url(url: &str) -> bool {
    let u = url.trim().to_ascii_lowercase();
    (u.starts_with("https://") || u.starts_with("http://")) && !u.contains(char::is_whitespace)
}

pub fn toast(cx: &mut App, text: impl Into<SharedString>) {
    toast_with_action(cx, text, None);
}

pub fn toast_with_action(
    cx: &mut App,
    text: impl Into<SharedString>,
    action: Option<(SharedString, Arc<dyn Fn(&mut Window, &mut App)>)>,
) {
    toast_with_actions(cx, text, action.into_iter().collect());
}

pub fn toast_with_actions(cx: &mut App, text: impl Into<SharedString>, actions: Vec<(SharedString, Arc<dyn Fn(&mut Window, &mut App)>)>) {
    let text: SharedString = crate::i18n::t(&text.into()).into();
    let mut actions: Vec<(SharedString, Arc<dyn Fn(&mut Window, &mut App)>)> = actions
        .into_iter()
        .map(|(l, f)| (crate::i18n::t(&l).into(), f))
        .collect();
    let action = if actions.is_empty() { None } else { Some(actions.remove(0)) };
    let t = cx.default_global::<Toasts>();
    t.next += 1;
    let id = t.next;
    t.items.push(Toast { id, text, action, extra: actions });
    if t.items.len() > 4 {
        t.items.remove(0);
    }
    cx.refresh_windows();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(std::time::Duration::from_secs(4)).await;
        let _ = cx.update(|cx| {
            cx.default_global::<Toasts>().items.retain(|x| x.id != id);
            cx.refresh_windows();
        });
    })
    .detach();
}

// ───────────────────────── small builders ─────────────────────────

/// Circular icon button (YouTube style).
pub fn icon_button(id: impl Into<ElementId>, name: &str, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size_10()
        .rounded_full()
        .cursor_pointer()
        .text_color(p.text)
        .hover(move |s| s.bg(p.hover))
        .child(icon(name).text_color(p.text))
}

/// Pill button with optional icon.
pub fn pill(id: impl Into<ElementId>, icon_name: Option<&str>, label: impl Into<SharedString>, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    let raw: SharedString = label.into();
    let label: SharedString = if raw.is_empty() { raw } else { crate::i18n::t(&raw).into() };
    div()
        .id(id.into())
        .flex()
        .flex_none()
        .items_center()
        .gap_2()
        .h_9()
        .px_4()
        .rounded_full()
        .bg(p.chip)
        .cursor_pointer()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .text_color(p.text)
        .hover(move |s| s.bg(p.hover))
        .when_some(icon_name.map(str::to_string), |d, n| d.child(icon(&n).size_4().text_color(p.text)))
        .when(!label.is_empty(), |d| d.child(label))
}

pub fn chip(id: impl Into<ElementId>, label: impl Into<SharedString>, active: bool, cx: &App) -> Stateful<Div> {
    let p = pal(cx);
    div()
        .id(id.into())
        .flex_none()
        .px_3()
        .py_1p5()
        .rounded_lg()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(active, |d| d.bg(p.chip_active_bg).text_color(p.chip_active_fg))
        .when(!active, move |d| d.bg(p.chip).text_color(p.text).hover(move |s| s.bg(p.hover)))
        .child(crate::i18n::t(&label.into()))
}

pub fn spinner(cx: &App) -> impl IntoElement {
    let p = pal(cx);
    div()
        .flex()
        .w_full()
        .justify_center()
        .py_8()
        .child(
            icon("loader-circle")
                .size_8()
                .text_color(p.text2)
                .with_animation(
                    "spin",
                    Animation::new(std::time::Duration::from_millis(900)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                ),
        )
}

pub fn empty_state(icon_name: &str, title: &str, subtitle: &str, cx: &App) -> Div {
    let p = pal(cx);
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .py_20()
        .w_full()
        .child(icon(icon_name).size_16().text_color(p.text3))
        .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).text_color(p.text).child(crate::i18n::t(title)))
        .child(div().text_sm().text_color(p.text2).max_w(px(480.)).text_center().child(crate::i18n::t(subtitle)))
}

pub fn error_state(err: &str, cx: &App) -> Div {
    empty_state("circle-alert", "Something went wrong", &friendly_error(err), cx)
}

/// One readable line instead of raw HTTP bodies or error chains.
pub fn friendly_error(err: &str) -> String {
    let lower = err.to_ascii_lowercase();
    let status = lower
        .split("http ")
        .nth(1)
        .and_then(|r| r.get(..3))
        .and_then(|c| c.parse::<u16>().ok());
    match status {
        Some(404) => return crate::i18n::t("This page isn't available."),
        Some(429) => return crate::i18n::t("YouTube is rate limiting requests. Try again in a minute."),
        Some(s) if (400..500).contains(&s) => {
            return if crate::i18n::is_tr() { format!("YouTube isteği reddetti (HTTP {s}).") } else { format!("YouTube rejected the request (HTTP {s}).") };
        }
        Some(s) if s >= 500 => {
            return if crate::i18n::is_tr() { format!("YouTube'da bir sorun oluştu (HTTP {s}). Tekrar deneyin.") } else { format!("YouTube had a problem (HTTP {s}). Try again.") };
        }
        _ => {}
    }
    if ["dns", "timed out", "timeout", "connect", "network", "resolve"].iter().any(|k| lower.contains(k)) {
        return crate::i18n::t("Couldn't reach YouTube. Check your connection.");
    }
    if lower.contains("recording is not available") {
        return crate::i18n::t("This live stream has ended and its recording isn't available.");
    }
    if lower.contains("premieres in") || lower.contains("live event will begin") {
        return crate::i18n::t("This hasn't started yet.");
    }
    if lower.contains("members-only") || lower.contains("join this channel") {
        return crate::i18n::t("This video is for channel members only.");
    }
    if lower.contains("private video") {
        return crate::i18n::t("This video is private.");
    }
    let line = err.lines().next().unwrap_or(err).trim();
    // which YouTube client failed is a detail, not news
    let line = ["visionOS: ", "Android VR: ", "Innertube: ", "yt-dlp: "]
        .iter()
        .fold(line, |l, p| l.strip_prefix(p).unwrap_or(l));
    if line.chars().count() > 160 { format!("{}…", line.chars().take(160).collect::<String>()) } else { line.to_string() }
}

pub fn avatar(url: Option<&str>, size: Pixels, name: &str, cx: &App) -> AnyElement {
    let p = pal(cx);
    if crate::store::settings_snapshot().hide_channel_avatars {
        return div().size(size).flex_none().into_any_element();
    }
    match url {
        Some(u) if !u.is_empty() => img(SharedString::from(crate::api::json::limit_avatar(u)))
            .size(size)
            .flex_none()
            .rounded_full()
            .object_fit(ObjectFit::Cover)
            .with_fallback({
                let n = name.chars().next().unwrap_or('?').to_uppercase().to_string();
                move || div().size_full().rounded_full().bg(rgb(0x5c6bc0)).flex().items_center().justify_center().text_color(white()).child(n.clone()).into_any_element()
            })
            .into_any_element(),
        _ => div()
            .size(size)
            .flex_none()
            .rounded_full()
            .bg(p.hover)
            .flex()
            .items_center()
            .justify_center()
            .text_color(p.text)
            .child(name.chars().next().unwrap_or('?').to_uppercase().to_string())
            .into_any_element(),
    }
}

pub fn copy_to_clipboard(text: String, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    toast(cx, "Link copied to clipboard");
}

/// Brand colors of the Stream Tube logo.
pub const BRAND_PINK: u32 = 0xF0307E;
pub const BRAND_VIOLET: u32 = 0x7A3CF5;

/// Font choices for the top-left brand wordmark.
pub fn brand_font_family(key: &str) -> Option<&'static str> {
    match key {
        "youtube" => Some(if cfg!(windows) { "Bahnschrift" } else { "Roboto Condensed" }),
        "segoe" => Some(if cfg!(windows) { "Segoe UI" } else { "DejaVu Sans" }),
        "arial" => Some("Arial"),
        "impact" => Some("Impact"),
        "verdana" => Some("Verdana"),
        "trebuchet" => Some("Trebuchet MS"),
        "georgia" => Some("Georgia"),
        "mono" => Some(if cfg!(windows) { "Consolas" } else { "DejaVu Sans Mono" }),
        _ => None,
    }
}

fn parse_hex_6(s: &str) -> Option<u32> {
    if s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()) {
        u32::from_str_radix(s, 16).ok()
    } else {
        None
    }
}

fn mc_color(code: char, default_color: Hsla, accent_color: Hsla) -> Option<Hsla> {
    let hex = match code.to_ascii_lowercase() {
        '0' => 0x000000,
        '1' => 0x0000AA,
        '2' => 0x00AA00,
        '3' => 0x00AAAA,
        '4' => 0xAA0000,
        '5' => 0xAA00AA,
        '6' => 0xFFAA00,
        '7' => 0xAAAAAA,
        '8' => 0x555555,
        '9' => 0x5555FF,
        'a' => 0x55FF55,
        'b' => 0x55FFFF,
        'c' => 0xFF5555,
        'd' => 0xFF55FF,
        'e' => 0xFFFF55,
        'f' => 0xFFFFFF,
        'r' => return Some(default_color),
        'x' => return Some(accent_color),
        _ => return None,
    };
    Some(rgb(hex).into())
}

/// Parses inline color codes in brand text:
/// - `[#RRGGBB]`, `[accent]`, `[reset]`
/// - `&#RRGGBB`, `&0`..`&f`, `&r` (reset), `&x` (accent)
fn parse_brand_segments(input: &str, default_color: Hsla, accent_color: Hsla) -> Vec<(String, Hsla)> {
    let mut out: Vec<(String, Hsla)> = Vec::new();
    let mut cur_color = default_color;
    let mut buf = String::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &input[i..];
        if rest.starts_with('[') {
            if let Some(end) = rest.find(']') {
                let tag = &rest[1..end];
                let parsed = if tag.eq_ignore_ascii_case("accent") {
                    Some(accent_color)
                } else if tag.eq_ignore_ascii_case("reset") || tag == "/" {
                    Some(default_color)
                } else if let Some(h) = tag.strip_prefix('#') {
                    parse_hex_6(h).map(|c| rgb(c).into())
                } else {
                    None
                };
                if let Some(col) = parsed {
                    if !buf.is_empty() {
                        out.push((std::mem::take(&mut buf), cur_color));
                    }
                    cur_color = col;
                    i += end + 1;
                    continue;
                }
            }
        } else if rest.starts_with("&#") && rest.len() >= 8 {
            if let Some(c) = parse_hex_6(&rest[2..8]) {
                if !buf.is_empty() {
                    out.push((std::mem::take(&mut buf), cur_color));
                }
                cur_color = rgb(c).into();
                i += 8;
                continue;
            }
        } else if (rest.starts_with('&') || rest.starts_with('§')) && rest.len() >= 2 {
            let prefix_len = rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            if let Some(code) = rest[prefix_len..].chars().next() {
                if let Some(col) = mc_color(code, default_color, accent_color) {
                    if !buf.is_empty() {
                        out.push((std::mem::take(&mut buf), cur_color));
                    }
                    cur_color = col;
                    i += prefix_len + code.len_utf8();
                    continue;
                }
            }
        }
        let ch = rest.chars().next().unwrap();
        buf.push(ch);
        i += ch.len_utf8();
    }
    if !buf.is_empty() {
        out.push((buf, cur_color));
    }
    out
}

/// Active brand title (plain text, with inline color codes stripped) for window title.
pub fn brand_title() -> String {
    let raw = crate::store::settings_snapshot().brand_name.trim().to_string();
    if raw.is_empty() {
        return "Stream Tube".into();
    }
    let plain: String = parse_brand_segments(&raw, white(), white()).into_iter().map(|(s, _)| s).collect();
    let plain = plain.trim().to_string();
    if plain.is_empty() { "Stream Tube".into() } else { plain }
}

/// The Stream Tube app logo (or user-selected custom logo).
pub fn logo(size: Pixels) -> Img {
    let s = crate::store::settings_snapshot();
    let custom = s.custom_logo.trim();
    if !custom.is_empty() && std::path::Path::new(custom).is_file() {
        let path = std::path::Path::new(custom);
        let raw_ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
        let ext = if raw_ext == "ico" { "png".to_string() } else { raw_ext };
        let meta = std::fs::metadata(path).ok();
        let mtime = meta
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let len = meta.map(|m| m.len()).unwrap_or(0);
        let key = format!("{custom}|{len}|{mtime}");
        let mut h: u64 = 14695981039346656037;
        for b in key.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(1099511628211);
        }
        return img(SharedString::from(format!("brand/custom-logo-{h:016x}.{ext}")))
            .size(size)
            .object_fit(ObjectFit::Contain)
            .flex_none();
    }
    let accent = app_icon::icon_accent(s.accent);
    img(SharedString::from(format!("brand/logo-{accent:06x}.svg"))).size(size).flex_none()
}

/// Brand wordmark next to the logo (supports custom font and inline color codes).
pub fn wordmark(cx: &App) -> Div {
    let p = pal(cx);
    let s = crate::store::settings_snapshot();
    let font = brand_font_family(&s.brand_font);
    let custom = s.brand_name.trim().to_string();
    if !custom.is_empty() {
        let segs = parse_brand_segments(&custom, p.text, p.accent);
        let mut d = div()
            .flex()
            .items_baseline()
            .text_lg()
            .font_weight(FontWeight::BOLD)
            .when_some(font, |d, f| d.font_family(f));
        for (text, col) in segs {
            d = d.child(div().text_color(col).child(text));
        }
        return d;
    }
    div()
        .flex()
        .items_baseline()
        .gap(px(5.))
        .text_lg()
        .font_weight(FontWeight::BOLD)
        .when_some(font, |d, f| d.font_family(f))
        .child(div().text_color(p.text).child("Stream"))
        .child(div().text_color(p.accent).child("Tube"))
}
