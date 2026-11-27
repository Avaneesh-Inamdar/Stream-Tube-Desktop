use crate::api::innertube;
use crate::i18n::{t, tf};
use crate::api::types::*;
use crate::store;
use crate::ui::app::{MenuItem, open_menu};
use crate::ui::cards::*;
use crate::ui::widgets::*;
use crate::ui::*;
use gpui_kit::component::input::{Input, InputEvent, InputState};

pub struct ChannelPage {
    id: String,
    data: Load<ChannelPage0>,
    tab: usize,
    tab_items: Load<Vec<Item>>,
    continuation: Option<String>,
    loading_more: bool,
    scroll: ScrollHandle,
    search: Entity<InputState>,
    searching: Option<String>,
    serial: u64,
    /// the channel description is shown in full
    about_open: bool,
    /// clicking a video toggles it for downloading instead of opening it
    selecting: bool,
    /// IDs of the videos picked, in the order they were picked
    selected: Vec<String>,
    _sub: Subscription,
}

#[derive(Clone)]
struct ChannelPage0 {
    info: ChannelItem,
    banner: Option<String>,
    tabs: Vec<ChannelTab>,
}

/// Tabs shown, by kind (see ChannelTab::kind), so channels work in every content language.
const WANTED_TABS: &[&str] = &["featured", "videos", "shorts", "streams", "podcasts", "releases", "playlists", "courses", "shows"];

impl ChannelPage {
    pub fn new(id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t("Search channel")));
        let sub = cx.subscribe_in(&search, window, |this: &mut Self, st, ev: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                let q = st.read(cx).value().to_string();
                if !q.trim().is_empty() {
                    this.search_channel(q, cx);
                }
            }
        });
        let mut s = Self {
            id,
            data: Load::Loading,
            tab: 0,
            tab_items: Load::Loading,
            continuation: None,
            loading_more: false,
            scroll: ScrollHandle::new(),
            search,
            searching: None,
            serial: 0,
            about_open: false,
            selecting: false,
            selected: Vec::new(),
            _sub: sub,
        };
        s.load(cx);
        s
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let id = self.id.clone();
        cx.spawn(async move |this, cx| {
            // Videos tab first (most useful), header comes with it
            let res = blocking(move || innertube::channel(&id, Some(crate::feed::VIDEOS_TAB))).await;
            let _ = this.update(cx, |this, cx| {
                match res {
                    Ok(cp) => {
                        let tabs: Vec<ChannelTab> = cp
                            .tabs
                            .into_iter()
                            .filter(|t| WANTED_TABS.contains(&t.kind.as_str()))
                            .filter(|t| !(store::settings_snapshot().hide_shorts && t.kind == "shorts"))
                            .collect();
                        this.tab = tabs.iter().position(|t| t.kind == "videos").unwrap_or(0);
                        this.tab_items = Load::Ready(cp.page.items);
                        this.continuation = cp.page.continuation;
                        if store::is_subscribed(&cp.info.id) {
                            store::subscribe(&cp.info.id, &cp.info.name, cp.info.avatar.as_deref());
                        }
                        this.data = Load::Ready(ChannelPage0 { info: cp.info, banner: cp.banner, tabs });
                    }
                    Err(e) => this.data = Load::Error(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        let Some(d) = self.data.ready() else { return };
        let Some(t) = d.tabs.get(idx).cloned() else { return };
        self.tab = idx;
        self.searching = None;
        self.stop_selecting();
        self.tab_items = Load::Loading;
        self.continuation = None;
        self.serial += 1;
        let serial = self.serial;
        let id = self.id.clone();
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::channel(&id, t.params.as_deref())).await;
            let _ = this.update(cx, |this, cx| {
                if this.serial != serial {
                    return;
                }
                match res {
                    Ok(cp) => {
                        this.tab_items = Load::Ready(cp.page.items);
                        this.continuation = cp.page.continuation;
                    }
                    Err(e) => this.tab_items = Load::Error(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn stop_selecting(&mut self) {
        self.selecting = false;
        self.selected.clear();
    }

    fn toggle_selected(&mut self, id: &str) {
        if let Some(i) = self.selected.iter().position(|s| s == id) {
            self.selected.remove(i);
        } else {
            self.selected.push(id.to_string());
        }
    }

    /// Queues the picked videos with the default download format.
    fn download_selected(&mut self, cx: &mut Context<Self>) {
        let s = store::settings_snapshot();
        let items = self.tab_items.ready().cloned().unwrap_or_default();
        // a filter may have hidden a picked video since (e.g. marked "Not interested")
        let videos: Vec<VideoItem> = self.selected.iter().filter_map(|id| items.iter().filter_map(Item::as_video).find(|v| v.id == *id && visible(v, &s)).cloned()).collect();
        if videos.is_empty() {
            return;
        }
        if !s.download_dir_chosen {
            // asked here rather than by start_many, so cancelling keeps the selection
            let rx = cx.prompt_for_paths(gpui::PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t("Save downloads here").into()) });
            cx.spawn(async move |this, cx| {
                let picked = rx.await.ok().and_then(|r| r.ok()).flatten().and_then(|p| p.into_iter().next());
                let _ = this.update(cx, |this, cx| match picked {
                    Some(dir) => {
                        store::update_settings(|s| {
                            s.download_dir = dir.to_string_lossy().into_owned();
                            s.download_dir_chosen = true;
                        });
                        this.stop_selecting();
                        crate::downloads::start_many(videos, cx);
                        cx.notify();
                    }
                    None => crate::ui::toast(cx, t("Download cancelled")),
                });
            })
            .detach();
            return;
        }
        self.stop_selecting();
        crate::downloads::start_many(videos, cx);
        cx.notify();
    }

    fn search_channel(&mut self, q: String, cx: &mut Context<Self>) {
        self.searching = Some(q.clone());
        self.stop_selecting();
        self.tab_items = Load::Loading;
        self.continuation = None;
        self.serial += 1;
        let serial = self.serial;
        let id = self.id.clone();
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::channel_search(&id, &q)).await;
            let _ = this.update(cx, |this, cx| {
                if this.serial != serial {
                    return;
                }
                match res {
                    Ok(p) => {
                        this.tab_items = Load::Ready(p.items);
                        this.continuation = p.continuation;
                    }
                    Err(e) => this.tab_items = Load::Error(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_more(&mut self, cx: &mut Context<Self>) {
        let Some(tok) = self.continuation.clone() else {
            self.loading_more = false;
            return;
        };
        let info = self.data.ready().map(|d| d.info.clone());
        cx.spawn(async move |this, cx| {
            let res = blocking(move || innertube::browse_continue(&tok)).await;
            let _ = this.update(cx, |this, cx| {
                this.loading_more = false;
                match res {
                    Ok(mut p) => {
                        if let Some(info) = &info {
                            for it in p.items.iter_mut() {
                                if let Item::Video(v) = it {
                                    if v.channel_name.is_empty() {
                                        v.channel_name = info.name.clone();
                                    }
                                    v.channel_id.get_or_insert(info.id.clone());
                                }
                            }
                        }
                        if let Some(items) = this.tab_items.ready_mut() {
                            items.extend(p.items);
                        }
                        this.continuation = p.continuation;
                    }
                    Err(_) => this.continuation = None,
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// Whether a video is shown on the page: not hidden by the user's filters, or from
/// a blocked channel (whose own page still shows its videos).
fn visible(v: &VideoItem, s: &store::Settings) -> bool {
    !store::is_filtered(v, s) || s.blocked_channels.iter().any(|(c, _)| Some(c) == v.channel_id.as_ref())
}

impl Render for ChannelPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        localize_placeholder(&self.search, "Search channel", window, cx);
        let p = pal(cx);
        let has_more = self.continuation.is_some();
        maybe_load_more(&self.scroll, &mut self.loading_more, has_more, cx, |this, cx| this.load_more(cx));
        let width = content_width(cx).min(px(1284.));
        let d = match &self.data {
            Load::Loading => return page_scroll("ch-scroll", &self.scroll).child(spinner(cx)).into_any_element(),
            Load::Error(e) => return page_scroll("ch-scroll", &self.scroll).child(error_state(e, cx)).into_any_element(),
            Load::Ready(d) => d.clone(),
        };
        let s = store::settings_snapshot();
        let subscribed = store::is_subscribed(&d.info.id);
        let blocked = s.blocked_channels.iter().any(|(i, _)| *i == d.info.id);
        let speed = store::channel_speed(&d.info.id);
        let info2 = d.info.clone();
        let header = div()
            .flex()
            .flex_col()
            .gap_4()
            .when_some(d.banner.clone(), |el, b| {
                el.child(div().w_full().h(width / 6.2).rounded_2xl().overflow_hidden().bg(p.surface).child(img(SharedString::from(b)).size_full().rounded_2xl().object_fit(ObjectFit::Cover)))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .py_2()
                    .child(avatar(d.info.avatar.as_deref(), px(160.), &d.info.name, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .flex_1()
                            .child(selectable_text(SharedString::from(format!("ch-name-{}", d.info.id)), d.info.name.clone()).text_3xl().font_weight(FontWeight::BOLD).text_color(p.text))
                            .child(
                                selectable_text(
                                    SharedString::from(format!("ch-meta-{}", d.info.id)),
                                    [d.info.handle.clone(), d.info.subscribers_text.clone().filter(|_| !store::settings_snapshot().hide_subscriber_counts), d.info.video_count_text.clone()]
                                        .into_iter()
                                        .flatten()
                                        .collect::<Vec<_>>()
                                        .join(" • "),
                                )
                                .text_sm()
                                .text_color(p.text2),
                            )
                            .when_some(d.info.description.clone().filter(|x| !x.trim().is_empty()), |el, desc| {
                                // two lines unless opened: some channels write pages here
                                let short = clamp_text(desc.trim(), 2, 180);
                                let long = short.is_some();
                                let open = self.about_open;
                                el.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .max_w(px(700.))
                                        .child(div().text_sm().text_color(p.text2).child(if open || !long {
                                            rich_text("ch-about", desc.trim().to_string(), cx)
                                        } else {
                                            selectable_text(SharedString::from(format!("ch-about-short-{}", d.info.id)), short.unwrap_or_default()).into_any_element()
                                        }))
                                        .when(long, |d| {
                                            d.child(
                                                div()
                                                    .id("ch-about-more")
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(p.text)
                                                    .cursor_pointer()
                                                    .child(if open { t("Show less") } else { t("Show more") })
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.about_open ^= true;
                                                        cx.notify();
                                                    })),
                                            )
                                        }),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .pt_2()
                                    .child(subscribe_button("ch-sub".into(), &d.info, subscribed, cx))
                                    .when_some(speed, |el, sp| el.child(pill("ch-speed", Some("gauge"), tf!("{0}x speed", sp), cx)))
                                    .child(pill("ch-more", Some("ellipsis"), "", cx).w_9().px_0().justify_center().on_click(move |ev, window, cx| {
                                        let info = info2.clone();
                                        let info3 = info2.clone();
                                        let id = info2.id.clone();
                                        let mut items = vec![
                                            MenuItem::new("link", t("Copy channel link"), move |_, cx| {
                                                copy_to_clipboard(format!("https://www.youtube.com/channel/{}", info.id), cx)
                                            }),
                                            MenuItem::new("globe", t("Open on YouTube"), move |_, _| {
                                                let _ = open::that(format!("https://www.youtube.com/channel/{id}"));
                                            }),
                                        ];
                                        if blocked {
                                            items.push(MenuItem::new("circle-check", t("Unblock channel"), move |_, cx| {
                                                let id = info3.id.clone();
                                                store::update_settings(|s| s.blocked_channels.retain(|(i, _)| *i != id));
                                                cx.refresh_windows();
                                            }));
                                        } else {
                                            items.push(MenuItem::new("ban", t("Block channel (hide everywhere)"), move |_, cx| {
                                                let (id, name) = (info3.id.clone(), info3.name.clone());
                                                store::update_settings(|s| s.blocked_channels.push((id, name)));
                                                cx.refresh_windows();
                                            }));
                                        }
                                        if store::speed_reset_available(&info2.id) {
                                            let id = info2.id.clone();
                                            items.push(MenuItem::new("gauge", t("Reset remembered speed"), move |_, cx| {
                                                store::clear_channel_speed(&id);
                                                cx.refresh_windows();
                                            }));
                                        }
                                        open_menu(ev.position(), items, window, cx);
                                    })),
                            ),
                    ),
            );
        let mut tabs = div().flex().items_center().gap_6().border_b_1().border_color(p.border);
        for (i, t) in d.tabs.iter().enumerate() {
            let active = i == self.tab && self.searching.is_none();
            tabs = tabs.child(
                div()
                    .id(SharedString::from(format!("ct-{i}")))
                    .py_3()
                    .text_base()
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .text_color(if active { p.text } else { p.text2 })
                    .when(active, |el| el.border_b_2().border_color(p.text))
                    .hover(move |s| s.text_color(p.text))
                    .child(crate::i18n::t(&t.title).to_string())
                    .on_click(cx.listener(move |this, _, _, cx| this.select_tab(i, cx))),
            );
        }
        tabs = tabs.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .w(px(220.))
                .child(icon("search").size_4().text_color(p.text2))
                .child(Input::new(&self.search).appearance(false).cleanable(true)),
        );
        let tab_kind = d.tabs.get(self.tab).map(|t| t.kind.clone()).unwrap_or_default();
        let has_videos = self.tab_items.ready().is_some_and(|items| items.iter().filter_map(Item::as_video).any(|v| visible(v, &s)));
        // picking videos to download: a toggle, then a bar with the count and actions.
        // Not on Home, whose shelves mix videos with playlists and channels.
        let select_bar = (has_videos && (tab_kind != "featured" || self.searching.is_some())).then(|| {
            let n = self.selected.len();
            let mut row = div().flex().items_center().gap_3();
            if self.selecting {
                row = row
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).text_color(p.text).child(tf!("{0} selected", n)))
                    .child(pill("ch-select-all", Some("square-check"), t("Select all"), cx).on_click(cx.listener(|this, _, _, cx| {
                        // only the videos shown, not ones hidden by filters
                        let s = store::settings_snapshot();
                        let ids: Vec<String> = this.tab_items.ready().into_iter().flatten().filter_map(Item::as_video).filter(|v| visible(v, &s)).map(|v| v.id.clone()).collect();
                        for id in ids {
                            if !this.selected.contains(&id) {
                                this.selected.push(id);
                            }
                        }
                        cx.notify();
                    })))
                    .child(
                        pill("ch-select-dl", Some("download"), t("Download"), cx)
                            .when(n == 0, |el| el.opacity(0.5))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.selected.is_empty() {
                                    this.download_selected(cx);
                                }
                            })),
                    )
                    .child(pill("ch-select-cancel", Some("x"), t("Cancel"), cx).on_click(cx.listener(|this, _, _, cx| {
                        this.stop_selecting();
                        cx.notify();
                    })));
            } else {
                row = row.child(pill("ch-select", Some("square-check"), t("Select"), cx).on_click(cx.listener(|this, _, _, cx| {
                    this.selecting = true;
                    cx.notify();
                })));
            }
            row
        });
        let selecting = self.selecting;
        let selected = self.selected.clone();
        // in select mode a cover over each video card takes the click and shows a checkbox
        let pickable = |card: AnyElement, item: &Item, w: Pixels, cx: &mut Context<Self>| -> AnyElement {
            let Some(v) = item.as_video().filter(|_| selecting) else { return card };
            let on = selected.contains(&v.id);
            let id = v.id.clone();
            div()
                .relative()
                .w(w)
                .child(card)
                .child(
                    div()
                        .id(SharedString::from(format!("ch-pick-{}", v.id)))
                        .absolute()
                        .inset_0()
                        // the card underneath gets no clicks or hover; the page still scrolls
                        .block_mouse_except_scroll()
                        // the cover hides the pointer from app-root, which handles these
                        .on_mouse_down(gpui::MouseButton::Navigate(gpui::NavigationDirection::Back), |_, window, cx| crate::ui::app::go_back(window, cx))
                        .on_mouse_down(gpui::MouseButton::Navigate(gpui::NavigationDirection::Forward), |_, window, cx| crate::ui::app::go_forward(window, cx))
                        .cursor_pointer()
                        .rounded_xl()
                        .when(on, |el| el.border_2().border_color(p.accent))
                        .child(
                            div()
                                .absolute()
                                .top_2()
                                .left_2()
                                .p_1()
                                .rounded_md()
                                .bg(gpui::black().opacity(0.6))
                                .child(icon(if on { "square-check" } else { "square" }).size_5().text_color(gpui::white())),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_selected(&id);
                            cx.notify();
                        })),
                )
                .into_any_element()
        };
        let body: AnyElement = match &self.tab_items {
            Load::Loading => spinner(cx).into_any_element(),
            Load::Error(e) => error_state(e, cx).into_any_element(),
            Load::Ready(items) => {
                let items: Vec<&Item> = items
                    .iter()
                    .filter(|i| match i {
                        Item::Video(v) => visible(v, &s),
                        _ => true,
                    })
                    .collect();
                if items.is_empty() {
                    empty_state("video-off", "This channel has no content here", "", cx).into_any_element()
                } else if tab_kind == "shorts" && self.searching.is_none() {
                    let (_, w) = grid_columns(width, 200.0, window.rem_size());
                    let cards = items
                        .iter()
                        .filter_map(|i| i.as_video().map(|v| (*i, v)))
                        .map(|(i, v)| {
                            let card = shorts_card(v, w, cx);
                            pickable(card, i, w, cx)
                        })
                        .collect();
                    grid(cards, width).into_any_element()
                } else {
                    let (_, w) = grid_columns(width, 280.0, window.rem_size());
                    let show_channel = tab_kind == "featured" || self.searching.is_some();
                    let cards = items
                        .iter()
                        .map(|i| {
                            let card = grid_item(i, w, show_channel, cx);
                            // video_card draws Shorts narrower than the column
                            let card_w = match i {
                                Item::Video(v) if v.is_short => w.min(px(240.)),
                                _ => w,
                            };
                            pickable(card, i, card_w, cx)
                        })
                        .collect();
                    let mut col = div().flex().flex_col().gap_6().child(grid(cards, width));
                    if self.loading_more {
                        col = col.child(spinner(cx));
                    }
                    col.into_any_element()
                }
            }
        };
        page_scroll("ch-scroll", &self.scroll)
            .child(
                div()
                    .flex()
                    .justify_center()
                    .px_6()
                    .pt_4()
                    .pb_24()
                    .child(div().w(width).flex().flex_col().gap_4().child(header).child(tabs).children(select_bar).child(body)),
            )
            .into_any_element()
    }
}
