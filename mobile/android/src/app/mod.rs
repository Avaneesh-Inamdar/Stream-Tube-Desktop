use crate::{
    entities::{Channel, Video},
    features::{
        account,
        library::{Language, Library},
        player,
        youtube::{self, Category, Client},
    },
};
use slint::{Model, SharedPixelBuffer, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

slint::include_modules!();

type State = Rc<RefCell<AppState>>;

struct AppState {
    library: Library,
    system_russian: bool,
    storage_writable: bool,
    request: Arc<AtomicU64>,
    query: String,
    channel: Option<Channel>,
    return_from_channel: Screen,
    account_request: i32,
    account_channels: Vec<Channel>,
}

enum Feed {
    Home(Category),
    Shorts,
    Following(Vec<Channel>),
    Channel(Channel),
    Search(String),
    Local(Vec<Video>),
}

pub fn run() -> Result<(), slint::PlatformError> {
    let window = StreamTube::new()?;
    let system_russian = sys_locale::get_locale()
        .is_some_and(|locale| locale.split(['-', '_']).next() == Some("ru"));
    let stored = Library::load();
    let storage_writable = stored.is_ok();
    let library = stored.unwrap_or_default();
    let russian = library.language.is_russian(system_russian);
    window.set_russian(russian);
    window.set_language_choice(language_index(library.language));
    window.set_thumbnails(library.thumbnails);
    window.set_account_available(account::available());
    let reconnect = library.connected_account;
    window.set_videos(Rc::new(VecModel::<VideoRow>::default()).into());
    if !storage_writable {
        window.set_notice(
            copy(
                russian,
                "Локальная библиотека недоступна",
                "Local library is unavailable",
            )
            .into(),
        );
    }

    let state = Rc::new(RefCell::new(AppState {
        library,
        system_russian,
        storage_writable,
        request: Arc::new(AtomicU64::new(0)),
        query: String::new(),
        channel: None,
        return_from_channel: Screen::Home,
        account_request: 0,
        account_channels: Vec::new(),
    }));
    bind_navigation(&window, &state);
    bind_videos(&window, &state);
    bind_settings(&window, &state);
    bind_account(&window, &state);
    reload(&window, &state);
    if reconnect && window.get_account_available() {
        authorize(&window, &state, false);
    }
    window.run()
}

fn bind_navigation(window: &StreamTube, state: &State) {
    let (target, app) = (window.as_weak(), state.clone());
    window.on_navigate(move |screen| {
        if let Some(ui) = target.upgrade() {
            if matches!(
                screen,
                Screen::Home | Screen::Shorts | Screen::Following | Screen::Library
            ) {
                ui.set_tab(screen);
            }
            ui.set_screen(screen);
            reload(&ui, &app);
        }
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_back(move || {
        if let Some(ui) = target.upgrade() {
            let screen = if ui.get_screen() == Screen::Channel {
                app.borrow().return_from_channel
            } else {
                ui.get_tab()
            };
            ui.set_screen(screen);
            reload(&ui, &app);
        }
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_reload(move || {
        if let Some(ui) = target.upgrade() {
            reload(&ui, &app);
        }
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_submit(move |input| {
        let Some(ui) = target.upgrade() else { return };
        let input = input.trim();
        if input.is_empty() {
            return;
        }
        if let Some(id) = youtube::video_id(input) {
            let title = copy(ui.get_russian(), "Видео YouTube", "YouTube video");
            play(
                &ui,
                &app,
                Video {
                    id,
                    title: title.into(),
                    ..Video::default()
                },
            );
            return;
        }
        if input.contains("://") || input.starts_with("www.") {
            ui.set_notice(
                copy(
                    ui.get_russian(),
                    "Нужна ссылка на видео YouTube",
                    "Enter a YouTube video link",
                )
                .into(),
            );
            return;
        }
        ui.set_notice("".into());
        app.borrow_mut().query = input.chars().take(200).collect();
        reload(&ui, &app);
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_open_channel(move |id, name| {
        let Some(ui) = target.upgrade() else { return };
        if !youtube::is_channel_id(&id) {
            return;
        }
        {
            let mut state = app.borrow_mut();
            if ui.get_screen() != Screen::Channel {
                state.return_from_channel = ui.get_screen();
            }
            state.channel = Some(Channel {
                id: id.to_string(),
                name: name.to_string(),
            });
        }
        ui.set_screen(Screen::Channel);
        reload(&ui, &app);
    });
}

fn bind_videos(window: &StreamTube, state: &State) {
    let (target, app) = (window.as_weak(), state.clone());
    window.on_watch(move |id| {
        let Some(ui) = target.upgrade() else { return };
        let video = find_video(&ui, &id);
        if let Some(video) = video {
            play(&ui, &app, video);
        }
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_save(move |id| {
        let Some(ui) = target.upgrade() else { return };
        let video = find_video(&ui, &id);
        let Some(video) = video else { return };
        edit_library(&ui, &app, |library| library.toggle_saved(video));
        if ui.get_screen() == Screen::Library && !ui.get_history_mode() {
            reload(&ui, &app);
        } else {
            let saved = app.borrow().library.is_saved(&id);
            let model = ui.get_videos();
            if let Some(index) = (0..model.row_count())
                .find(|index| model.row_data(*index).is_some_and(|row| row.id == id))
            {
                if let Some(mut row) = model.row_data(index) {
                    row.saved = saved;
                    model.set_row_data(index, row);
                }
            }
        }
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_follow(move || {
        let Some(ui) = target.upgrade() else { return };
        let channel = app.borrow().channel.clone();
        let Some(channel) = channel else { return };
        if app
            .borrow()
            .account_channels
            .iter()
            .any(|remote| remote.id == channel.id)
        {
            return;
        }
        let id = channel.id.clone();
        edit_library(&ui, &app, |library| library.toggle_following(channel));
        ui.set_channel_followed(app.borrow().library.is_following(&id));
    });
}

fn bind_settings(window: &StreamTube, state: &State) {
    let (target, app) = (window.as_weak(), state.clone());
    window.on_choose_language(move |choice| {
        let Some(ui) = target.upgrade() else { return };
        let language = match choice {
            1 => Language::English,
            2 => Language::Russian,
            _ => Language::System,
        };
        edit_library(&ui, &app, |library| library.language = language);
        let state = app.borrow();
        ui.set_language_choice(language_index(state.library.language));
        ui.set_russian(state.library.language.is_russian(state.system_russian));
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_toggle_thumbnails(move || {
        let Some(ui) = target.upgrade() else { return };
        edit_library(&ui, &app, |library| {
            library.thumbnails = !library.thumbnails
        });
        let enabled = app.borrow().library.thumbnails;
        ui.set_thumbnails(enabled);
    });

    let target = window.as_weak();
    window.on_open_source(move || {
        if let Some(ui) = target.upgrade() {
            if rustferry::system::open_url(
                "https://github.com/ShiroKSH/Stream-Tube-Desktop/tree/android/mobile/android",
            )
            .is_err()
            {
                ui.set_notice(
                    copy(
                        ui.get_russian(),
                        "Не удалось открыть исходный код",
                        "Could not open the source",
                    )
                    .into(),
                );
            }
        }
    });
}

fn bind_account(window: &StreamTube, state: &State) {
    let target = window.as_weak();
    account::listen(move |request, status, name, channels| {
        let _ = target.upgrade_in_event_loop(move |ui| {
            ui.invoke_account_result(request, status, name.into(), channels.into());
        });
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_authorize_account(move || {
        if let Some(ui) = target.upgrade() {
            authorize(&ui, &app, true);
        }
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_disconnect_account(move || {
        let Some(ui) = target.upgrade() else { return };
        {
            let mut state = app.borrow_mut();
            state.account_request = state.account_request.wrapping_add(1);
            state.account_channels.clear();
        }
        account::cancel();
        edit_library(&ui, &app, |library| library.connected_account = false);
        ui.set_account_busy(false);
        ui.set_account_connected(false);
        ui.set_account_name("".into());
        ui.set_account_message("".into());
        if ui.get_screen() == Screen::Following {
            reload(&ui, &app);
        }
    });

    let (target, app) = (window.as_weak(), state.clone());
    window.on_account_result(move |request, status, name, channels| {
        let Some(ui) = target.upgrade() else { return };
        if request != app.borrow().account_request {
            return;
        }
        ui.set_account_busy(false);
        let russian = ui.get_russian();
        if status == 0 {
            let Ok(channels) = serde_json::from_str::<Vec<Channel>>(&channels) else {
                ui.set_account_message(
                    copy(
                        russian,
                        "Не удалось прочитать подписки",
                        "Could not read subscriptions",
                    )
                    .into(),
                );
                return;
            };
            app.borrow_mut().account_channels = channels
                .into_iter()
                .filter(|channel| youtube::is_channel_id(&channel.id))
                .take(500)
                .collect();
            edit_library(&ui, &app, |library| library.connected_account = true);
            ui.set_account_name(name);
            ui.set_account_connected(true);
            ui.set_account_message("".into());
            if ui.get_screen() == Screen::Following {
                reload(&ui, &app);
            }
            return;
        }
        let message = match status {
            1 => copy(russian, "Вход отменён", "Sign-in cancelled"),
            3 => copy(
                russian,
                "Google не разрешил доступ этому приложению",
                "Google did not authorize this app",
            ),
            4 => copy(
                russian,
                "Не удалось связаться с Google",
                "Could not reach Google",
            ),
            5 => copy(
                russian,
                "Подключите аккаунт YouTube снова",
                "Reconnect your YouTube account",
            ),
            _ => copy(
                russian,
                "Вход через Google сейчас недоступен",
                "Google sign-in is unavailable",
            ),
        };
        ui.set_account_message(message.into());
    });
}

fn authorize(ui: &StreamTube, state: &State, interactive: bool) {
    let request = {
        let mut state = state.borrow_mut();
        state.account_request = state.account_request.wrapping_add(1);
        state.account_request
    };
    ui.set_account_busy(true);
    ui.set_account_message("".into());
    if !account::start(request, interactive) {
        ui.set_account_busy(false);
        ui.set_account_message(
            copy(
                ui.get_russian(),
                "Вход через Google недоступен",
                "Google sign-in is unavailable",
            )
            .into(),
        );
    }
}

fn following_channels(state: &AppState) -> Vec<Channel> {
    let mut ids = std::collections::HashSet::new();
    state
        .library
        .following
        .iter()
        .chain(&state.account_channels)
        .filter(|channel| ids.insert(channel.id.as_str()))
        .cloned()
        .collect()
}

fn reload(ui: &StreamTube, state: &State) {
    let russian = ui.get_russian();
    let screen = ui.get_screen();
    let (feed, empty, request, generation, thumbnails, saved) = {
        let state = state.borrow();
        let generation = state.request.fetch_add(1, Ordering::Relaxed) + 1;
        let feed = match screen {
            Screen::Home => Some(Feed::Home(match ui.get_category() {
                1 => Category::Music,
                2 => Category::Games,
                3 => Category::Tech,
                4 => Category::Live,
                _ => Category::All,
            })),
            Screen::Shorts => Some(Feed::Shorts),
            Screen::Following if !ui.get_channels_mode() => {
                let channels = following_channels(&state);
                (!channels.is_empty()).then_some(Feed::Following(channels))
            }
            Screen::Library => Some(Feed::Local(if ui.get_history_mode() {
                state.library.history.clone()
            } else {
                state.library.saved.clone()
            })),
            Screen::Search if !state.query.is_empty() => Some(Feed::Search(state.query.clone())),
            Screen::Channel => state.channel.clone().map(Feed::Channel),
            _ => None,
        };
        if screen == Screen::Following {
            let channels = following_channels(&state)
                .into_iter()
                .map(|channel| ChannelRow {
                    id: channel.id.clone().into(),
                    name: channel.name.clone().into(),
                })
                .collect::<Vec<_>>();
            ui.set_channels(Rc::new(VecModel::from(channels)).into());
        }
        let empty = match screen {
            Screen::Following => copy(
                russian,
                "Откройте канал под видео и подпишитесь",
                "Open a channel under a video to follow it",
            ),
            Screen::Library if ui.get_history_mode() => copy(
                russian,
                "Здесь появятся открытые видео",
                "Videos you open will appear here",
            ),
            Screen::Library => copy(
                russian,
                "Сохраните видео из любой ленты",
                "Save a video from any feed",
            ),
            Screen::Search if state.query.is_empty() => copy(
                russian,
                "Введите название видео или канала",
                "Search by video or channel name",
            ),
            _ => copy(russian, "Видео не найдены", "No videos found"),
        };
        if let Some(Feed::Channel(channel)) = &feed {
            ui.set_channel_name(channel.name.clone().into());
            ui.set_channel_followed(state.library.is_following(&channel.id));
            ui.set_channel_remote(
                state
                    .account_channels
                    .iter()
                    .any(|remote| remote.id == channel.id),
            );
        }
        (
            feed,
            empty,
            state.request.clone(),
            generation,
            state.library.thumbnails,
            state
                .library
                .saved
                .iter()
                .map(|video| video.id.clone())
                .collect::<std::collections::HashSet<_>>(),
        )
    };

    replace_rows(ui, Vec::new());
    ui.set_failed(false);
    ui.set_busy(false);
    ui.set_message(if screen == Screen::Settings {
        "".into()
    } else {
        empty.into()
    });
    let Some(feed) = feed else { return };
    ui.set_busy(true);
    ui.set_message(copy(russian, "Загрузка…", "Loading…").into());

    let target = ui.as_weak();
    let transaction = FeedTransaction {
        target,
        request,
        generation,
        thumbnails,
        saved,
        russian,
        empty,
    };
    let result = std::thread::Builder::new()
        .name("stream-tube-feed".into())
        .spawn(move || {
            fetch_feed(feed, transaction);
        });
    if result.is_err() {
        feed_error(ui);
    }
}

struct FeedTransaction {
    target: slint::Weak<StreamTube>,
    request: Arc<AtomicU64>,
    generation: u64,
    thumbnails: bool,
    saved: std::collections::HashSet<String>,
    russian: bool,
    empty: &'static str,
}

fn fetch_feed(feed: Feed, transaction: FeedTransaction) {
    let FeedTransaction {
        target,
        request,
        generation,
        thumbnails,
        saved,
        russian,
        empty,
    } = transaction;
    let client = Client::new(russian);
    let videos = match feed {
        Feed::Home(category) => client.home(category),
        Feed::Shorts => client.shorts(),
        Feed::Following(channels) => follow_feed(russian, &channels, &request, generation),
        Feed::Channel(channel) => client.channel(&channel),
        Feed::Search(query) => client.search(&query, false),
        Feed::Local(videos) => Some(videos),
    };
    let ids = videos
        .iter()
        .flatten()
        .map(|video| video.id.clone())
        .collect::<Vec<_>>();
    let check = request.clone();
    if target
        .upgrade_in_event_loop(move |ui| {
            if check.load(Ordering::Relaxed) != generation {
                return;
            }
            ui.set_busy(false);
            let Some(videos) = videos else {
                feed_error(&ui);
                return;
            };
            if videos.is_empty() {
                ui.set_message(empty.into());
                return;
            }
            ui.set_message("".into());
            replace_rows(
                &ui,
                videos
                    .iter()
                    .map(|video| video_row(video, saved.contains(&video.id)))
                    .collect(),
            );
        })
        .is_err()
        || !thumbnails
    {
        return;
    }

    let http = youtube::thumbnail_agent();
    for (index, id) in ids.into_iter().enumerate() {
        if request.load(Ordering::Relaxed) != generation {
            break;
        }
        let Some(rgba) = youtube::thumbnail(&http, &id) else {
            continue;
        };
        let check = request.clone();
        let _ = target.upgrade_in_event_loop(move |ui| {
            if check.load(Ordering::Relaxed) != generation {
                return;
            }
            let model = ui.get_videos();
            if let Some(mut row) = model.row_data(index) {
                let pixels =
                    SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height());
                row.thumbnail = slint::Image::from_rgba8(pixels);
                row.has_thumbnail = true;
                model.set_row_data(index, row);
            }
        });
    }
}

fn follow_feed(
    russian: bool,
    channels: &[Channel],
    request: &AtomicU64,
    generation: u64,
) -> Option<Vec<Video>> {
    let mut result = Vec::new();
    let mut received = false;
    for batch in channels.chunks(6) {
        if request.load(Ordering::Relaxed) != generation {
            return None;
        }
        std::thread::scope(|scope| {
            let feeds = batch
                .iter()
                .map(|channel| scope.spawn(move || Client::new(russian).channel(channel)))
                .collect::<Vec<_>>();
            let mut recent = Vec::new();
            for feed in feeds {
                if let Ok(Some(videos)) = feed.join() {
                    received = true;
                    recent.push(videos.into_iter());
                }
            }
            for _ in 0..4 {
                result.extend(recent.iter_mut().filter_map(Iterator::next));
            }
        });
    }
    received.then_some(result)
}

fn replace_rows(ui: &StreamTube, rows: Vec<VideoRow>) {
    let model = ui.get_videos();
    if let Some(model) = model.as_any().downcast_ref::<VecModel<VideoRow>>() {
        model.set_vec(rows);
    }
}

fn video_row(video: &Video, saved: bool) -> VideoRow {
    VideoRow {
        id: video.id.clone().into(),
        title: video.title.clone().into(),
        channel: video.channel.name.clone().into(),
        channel_id: video.channel.id.clone().into(),
        duration: video.duration.clone().into(),
        views: video.views.clone().into(),
        published: video.published.clone().into(),
        saved,
        short: video.short,
        ..Default::default()
    }
}

fn find_video(ui: &StreamTube, id: &str) -> Option<Video> {
    let row = ui.get_videos().iter().find(|row| row.id == id)?;
    Some(Video {
        id: row.id.to_string(),
        title: row.title.to_string(),
        channel: Channel {
            id: row.channel_id.to_string(),
            name: row.channel.to_string(),
        },
        duration: row.duration.to_string(),
        views: row.views.to_string(),
        published: row.published.to_string(),
        short: row.short,
    })
}

fn play(ui: &StreamTube, state: &State, video: Video) {
    if player::open(
        &video.id,
        &video.title,
        &video.channel.name,
        ui.get_russian(),
        video.short,
    )
    .is_err()
    {
        ui.set_notice(
            copy(
                ui.get_russian(),
                "Не удалось открыть плеер",
                "Could not open the player",
            )
            .into(),
        );
        return;
    }
    edit_library(ui, state, |library| library.watched(video));
}

fn edit_library(ui: &StreamTube, state: &State, edit: impl FnOnce(&mut Library)) {
    let mut state = state.borrow_mut();
    let mut updated = state.library.clone();
    edit(&mut updated);
    if !state.storage_writable || updated.write().is_err() {
        ui.set_notice(
            copy(
                ui.get_russian(),
                "Не удалось сохранить изменения",
                "Could not save changes",
            )
            .into(),
        );
        return;
    }
    state.library = updated;
}

fn feed_error(ui: &StreamTube) {
    ui.set_busy(false);
    ui.set_failed(true);
    ui.set_message(
        copy(
            ui.get_russian(),
            "Не удалось загрузить видео",
            "Could not load videos",
        )
        .into(),
    );
}

fn language_index(language: Language) -> i32 {
    match language {
        Language::System => 0,
        Language::English => 1,
        Language::Russian => 2,
    }
}

fn copy(russian: bool, ru: &'static str, en: &'static str) -> &'static str {
    if russian { ru } else { en }
}
