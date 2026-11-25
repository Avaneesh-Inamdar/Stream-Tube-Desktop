//! Keep the opening media for the next Shorts in memory until playback uses it.
use super::proxy;
use crate::api::{streams, types::VideoItem};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};

pub const MAX_AHEAD: u8 = 5;

const PLAYLIST_LIMIT: usize = 512 * 1024;
const SEGMENT_LIMIT: usize = 4 * 1024 * 1024;
const STREAM_PREFIX: usize = 512 * 1024;
const PER_SHORT_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone)]
pub(super) struct Media {
    pub bytes: Arc<[u8]>,
    pub total: u64,
    pub mime: String,
}

type Assets = Mutex<HashMap<String, Media>>;

#[derive(Default)]
struct Preloads {
    upcoming: Vec<String>,
    videos: HashMap<String, Arc<Assets>>,
    quality: u32,
    audio_only: bool,
    running: bool,
}

static STATE: OnceLock<Mutex<Preloads>> = OnceLock::new();

pub fn clear() {
    if let Some(state) = STATE.get() {
        let mut state = state.lock();
        state.upcoming.clear();
        state.videos.clear();
    }
}

pub fn update(feed: &[VideoItem], quality: u32, audio_only: bool) {
    let ahead = crate::store::settings_snapshot().shorts_preload.min(MAX_AHEAD) as usize;
    if ahead == 0 || feed.is_empty() {
        clear();
        return;
    }
    let current = &feed[0].id;
    let next = &feed[1..feed.len().min(1 + ahead)];
    let state = STATE.get_or_init(Default::default);
    let mut s = state.lock();
    if (s.quality, s.audio_only) != (quality, audio_only) {
        s.videos.clear();
        s.quality = quality;
        s.audio_only = audio_only;
    }
    s.videos.retain(|id, _| id == current || next.iter().any(|v| v.id == *id));
    if !s.upcoming.iter().eq(next.iter().map(|v| &v.id)) {
        s.upcoming = next.iter().map(|v| v.id.clone()).collect();
    }
    if s.running || s.upcoming.iter().all(|id| s.videos.contains_key(id)) {
        return;
    }
    s.running = true;
    // One worker always takes the nearest missing Short from the latest feed position.
    std::thread::spawn(move || {
        loop {
            let (id, assets, quality, audio_only) = {
                let mut s = state.lock();
                let Some(id) = s.upcoming.iter().find(|id| !s.videos.contains_key(*id)).cloned() else {
                    s.running = false;
                    return;
                };
                let assets = Arc::downgrade(s.videos.entry(id.clone()).or_default());
                (id, assets, s.quality, s.audio_only)
            };
            preload(&id, assets, quality, audio_only);
        }
    });
}

pub(super) fn cached(url: &str) -> Option<Media> {
    STATE
        .get()?
        .lock()
        .videos
        .values()
        .find_map(|assets| assets.lock().get(url).cloned())
}

/// The stream resolver already fetches the master to list qualities and audio tracks.
pub(crate) fn remember_master(id: &str, url: &str, text: &str) {
    if text.len() > PLAYLIST_LIMIT || !text.starts_with("#EXTM3U") {
        return;
    }
    let Some(assets) = STATE.get().and_then(|s| s.lock().videos.get(id).cloned()) else {
        return;
    };
    let media = Media {
        bytes: Arc::from(text.as_bytes()),
        total: text.len() as u64,
        mime: "application/vnd.apple.mpegurl".into(),
    };
    Download(Arc::downgrade(&assets)).store(url, media);
}

struct Download(Weak<Assets>);

impl Download {
    fn store(&self, url: &str, media: Media) {
        if let Some(assets) = self.0.upgrade() {
            let mut assets = assets.lock();
            let used: usize = assets.values().map(|a| a.bytes.len()).sum();
            if used + media.bytes.len() <= PER_SHORT_LIMIT {
                assets.insert(url.to_owned(), media);
            }
        }
    }

    fn fetch(&self, url: &str, limit: usize, prefix: bool) -> Option<Media> {
        if self.0.strong_count() == 0 || !url.starts_with("https://") {
            return None;
        }
        let mut req = proxy::agent().get(url).header("User-Agent", crate::api::http::BROWSER_UA);
        if prefix {
            req = req.header("Range", &format!("bytes=0-{}", limit - 1));
        }
        let mut resp = req.call().ok()?;
        let range = match resp.status().as_u16() {
            200 => None,
            206 if prefix => {
                let (end, total) = resp
                    .headers()
                    .get("Content-Range")?
                    .to_str()
                    .ok()?
                    .strip_prefix("bytes 0-")?
                    .split_once('/')?;
                Some((end.parse::<u64>().ok()?.checked_add(1)?, total.parse::<u64>().ok()?))
            }
            _ => return None,
        };
        let mime = resp
            .headers()
            .get("Content-Type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let bytes = resp.body_mut().with_config().limit(limit as u64).read_to_vec().ok()?;
        let len = bytes.len() as u64;
        let (expected, total) = range.unwrap_or((len, len));
        if len == 0 || len != expected || total < len {
            return None;
        }
        Some(Media {
            bytes: bytes.into(),
            total,
            mime,
        })
    }

    fn playlist(&self, url: &str) -> Option<String> {
        if let Some(media) = self.0.upgrade()?.lock().get(url).cloned() {
            return std::str::from_utf8(&media.bytes).ok().map(str::to_owned);
        }
        let media = self.fetch(url, PLAYLIST_LIMIT, false)?;
        let text = std::str::from_utf8(&media.bytes).ok()?.to_owned();
        if !text.starts_with("#EXTM3U") {
            return None;
        }
        self.store(url, media);
        Some(text)
    }

    fn opening_segments(&self, playlist: &str) {
        for line in playlist.lines().map(str::trim) {
            let uri = if line.starts_with("#EXT-X-MAP:") || line.starts_with("#EXT-X-KEY:") {
                proxy::hls_attr(line, "URI")
            } else if !line.is_empty() && !line.starts_with('#') {
                Some(line.to_owned())
            } else {
                continue;
            };
            if let Some(url) = uri {
                if let Some(media) = self.fetch(&url, SEGMENT_LIMIT, false) {
                    self.store(&url, media);
                }
            }
            if !line.starts_with('#') {
                break;
            }
        }
    }
}

fn preload(id: &str, assets: Weak<Assets>, quality: u32, audio_only: bool) {
    let Ok(info) = streams::get_streams(id) else { return };
    if info.is_live || assets.strong_count() == 0 {
        return;
    }
    let download = Download(assets);
    if info.hls_only {
        let Some(master) = info.hls_url.as_deref().and_then(|url| download.playlist(url)) else {
            return;
        };
        let audio = streams::hls_audio_choice(&info, None);
        if let Some(selected) = proxy::select_master(&master, quality, audio.as_deref()) {
            for line in selected.lines() {
                let url = if line.starts_with("#EXT-X-MEDIA:") {
                    proxy::hls_attr(line, "URI")
                } else if !line.starts_with('#') && !line.is_empty() {
                    Some(line.to_owned())
                } else {
                    None
                };
                if let Some(playlist) = url.and_then(|url| download.playlist(&url)) {
                    download.opening_segments(&playlist);
                }
            }
        } else {
            download.opening_segments(&master);
        }
    } else {
        let settings = crate::store::settings_snapshot();
        let (video, audio) = streams::pick_formats(&info, quality, &settings.codec_pref, None);
        for format in [video.filter(|_| !audio_only), audio].into_iter().flatten() {
            if format.url.contains("googlevideo.com") {
                if let Some(media) = download.fetch(&format.url, STREAM_PREFIX, true) {
                    download.store(&format.url, media);
                }
            }
        }
    }
}
