//! Dynamic MPEG-DASH live proxy for YouTube segmented live adaptive formats.
//!
//! YouTube's HLS manifests (`VISIONOS`, `ANDROID_VR`, `MWEB`) cap live broadcasts
//! at 1080p60 (`avc1`), whereas `TVHTML5` (`5.20251120`) exposes full `1440p60`
//! (`itag 308` VP9 / `itag 400` AV1) and `2160p` (`itag 313`/`315` VP9 / `itag 401`
//! AV1) live adaptive formats alongside `itag 140` (AAC) / `itag 251` (Opus) audio.
//! Once the `n` challenge is solved, every `&sq=<N>` request returns a full,
//! self-initializing WebM or fragmented MP4 segment and reports the current live
//! edge in `X-Head-Seqnum`.
//!
//! This module serves a minimal MPEG-DASH dynamic MPD (`isoff-live:2011`) on a
//! token-authenticated loopback HTTP server and streams `&sq=<N>` segments straight
//! to libmpv while continuously re-anchoring the live head from upstream headers.
use crate::api::http;
use crate::api::types::StreamFormat;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Extra per-format metadata parsed from Innertube's live `adaptiveFormats` JSON.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveFormatMeta {
    pub target_duration_secs: u64,
    pub max_dvr_secs: u64,
    pub audio_sample_rate: u32,
}

impl Default for LiveFormatMeta {
    fn default() -> Self {
        Self {
            target_duration_secs: 5,
            max_dvr_secs: 0,
            audio_sample_rate: 44100,
        }
    }
}

static FORMAT_META: Mutex<VecDeque<(String, LiveFormatMeta)>> = Mutex::new(VecDeque::new());
static HEAD_CACHE: Mutex<VecDeque<(String, u64, Instant)>> = Mutex::new(VecDeque::new());

const HEAD_CACHE_TTL: Duration = Duration::from_secs(30);
/// Cap the live DVR seekbar window to 4 hours (matches YouTube's standard desktop DVR bar).
const MAX_DVR_WINDOW_SECS: u64 = 4 * 3600;

/// Stable key shared across all video/audio itags of the same live broadcast.
fn live_cache_key(url: &str) -> String {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some((_, id)) = parsed.query_pairs().find(|(k, _)| k == "id") {
            if !id.is_empty() {
                return id.into_owned();
            }
        }
    }
    url.to_owned()
}

fn remember_head(key: &str, head_sq: u64) {
    let now = Instant::now();
    let mut cache = HEAD_CACHE.lock();
    if let Some((_, existing, at)) = cache.iter_mut().find(|(k, _, _)| k == key) {
        if head_sq >= *existing || at.elapsed() > Duration::from_secs(10) {
            *existing = head_sq;
            *at = now;
        }
        return;
    }
    while cache.len() >= 128 {
        cache.pop_front();
    }
    cache.push_back((key.to_owned(), head_sq, now));
}

fn cached_head(url: &str, dur_sec: u64) -> Option<u64> {
    let key = live_cache_key(url);
    let dur_sec = dur_sec.clamp(1, 10);
    let cache = HEAD_CACHE.lock();
    let (_, sq, at) = cache.iter().rev().find(|(k, _, _)| k == &key)?;
    let elapsed = at.elapsed();
    if elapsed > HEAD_CACHE_TTL {
        return None;
    }
    Some(sq.saturating_add(elapsed.as_secs() / dur_sec))
}

/// Remembers `targetDurationSec`, `maxDvrDurationSec` and `audioSampleRate` for
/// a live adaptive stream URL without changing `StreamFormat`'s shape.
pub fn remember_format_meta(url: &str, meta: LiveFormatMeta) {
    let mut cache = FORMAT_META.lock();
    if let Some((_, existing)) = cache.iter_mut().find(|(u, _)| u == url) {
        *existing = meta;
        return;
    }
    while cache.len() >= 512 {
        cache.pop_front();
    }
    cache.push_back((url.to_owned(), meta));
}

fn format_meta(url: &str) -> LiveFormatMeta {
    FORMAT_META
        .lock()
        .iter()
        .rev()
        .find(|(u, _)| u == url)
        .map(|(_, m)| *m)
        .unwrap_or_default()
}

/// Whether `url` is a YouTube segmented live adaptive stream (`&sq=<N>`).
pub fn is_live_dash_url(url: &str) -> bool {
    if !url.contains("googlevideo.com") {
        return false;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let mut has_noclen = false;
    let mut has_live = false;
    for (k, v) in parsed.query_pairs() {
        if k == "noclen" && v == "1" {
            has_noclen = true;
        } else if (k == "live" && v == "1") || (k == "source" && v == "yt_live_broadcast") {
            has_live = true;
        }
    }
    has_noclen && has_live
}

fn agent() -> ureq::Agent {
    static AGENT: http::NetworkAgent = http::NetworkAgent::new();
    AGENT.get(&crate::store::settings_snapshot(), |builder| {
        builder
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(6)))
            .timeout_recv_response(Some(Duration::from_secs(6)))
            .timeout_recv_body(Some(Duration::from_secs(30)))
    })
}

fn parse_head_seqnum(resp: &ureq::http::Response<ureq::Body>) -> Option<u64> {
    resp.headers()
        .get("x-head-seqnum")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

fn append_query(base: &str, extra: &str) -> String {
    if base.contains('?') {
        format!("{base}&{extra}")
    } else {
        format!("{base}?{extra}")
    }
}

/// Probes the current live edge sequence number (`X-Head-Seqnum`) with a zero-byte
/// `HEAD` request (falling back to a 1-byte `GET &headm=4&range=0-0`).
pub fn probe_head(url: &str) -> Option<u64> {
    let ag = agent();
    let key = live_cache_key(url);
    if let Ok(resp) = ag.head(url).header("User-Agent", http::BROWSER_UA).call() {
        if resp.status().as_u16() < 400 {
            if let Some(sq) = parse_head_seqnum(&resp) {
                remember_head(&key, sq);
                return Some(sq);
            }
        }
    }
    let fallback = append_query(url, "headm=4&range=0-0");
    if let Ok(resp) = ag.get(&fallback).header("User-Agent", http::BROWSER_UA).call() {
        if resp.status().as_u16() < 400 {
            if let Some(sq) = parse_head_seqnum(&resp) {
                remember_head(&key, sq);
                return Some(sq);
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct TrackSpec {
    pub itag: u32,
    pub url: String,
    pub mime: String,
    pub codecs: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate: u64,
    pub sample_rate: u32,
}

impl TrackSpec {
    fn from_format(f: &StreamFormat) -> Self {
        let meta = format_meta(&f.url);
        let mime = if f.mime.is_empty() {
            if f.is_audio { "audio/mp4".into() } else { "video/webm".into() }
        } else {
            f.mime.clone()
        };
        Self {
            itag: f.itag,
            url: f.url.clone(),
            mime,
            codecs: f.codecs.clone(),
            width: f.width,
            height: f.height,
            fps: f.fps,
            bitrate: f.bitrate,
            sample_rate: meta.audio_sample_rate,
        }
    }

    fn ext(&self) -> &'static str {
        if self.mime.contains("webm") { "webm" } else { "m4s" }
    }
}

struct HeadTracker {
    head_sq: u64,
    observed_at: Instant,
}

struct LiveSession {
    cache_key: String,
    video: Option<TrackSpec>,
    audio: Option<TrackSpec>,
    dur_sec: u64,
    /// Maximum DVR window in segments (0 when DVR is disabled by the broadcast).
    max_dvr_segs: u64,
    /// Requested offset behind live edge when this session opened (seconds).
    start_behind_secs: u64,
    head: Mutex<HeadTracker>,
    /// `(served_sq, head_sq_at_serve)` of the first primary media segment served.
    first_served: Mutex<Option<(u64, u64)>>,
    /// Set when upstream rejects media segments (HTTP 403 or repeated non-edge errors)
    /// so playback can fail fast and fall back to HLS.
    fatal_error: AtomicBool,
    consecutive_failures: AtomicU32,
}

impl LiveSession {
    fn new(
        cache_key: String,
        video: Option<TrackSpec>,
        audio: Option<TrackSpec>,
        head_sq: u64,
        meta: LiveFormatMeta,
        behind_secs: f64,
    ) -> Self {
        let dur_sec = meta.target_duration_secs.clamp(1, 10);
        // Leave a small 2-segment safety margin from the eviction edge so seeking
        // to the far left of the DVR bar still lands on an available segment.
        let max_dvr_segs = if meta.max_dvr_secs > 0 {
            (meta.max_dvr_secs / dur_sec)
                .saturating_sub(2)
                .min(MAX_DVR_WINDOW_SECS / dur_sec)
        } else {
            0
        };
        let max_behind = max_dvr_segs.min(head_sq).saturating_mul(dur_sec);
        let start_behind_secs = if behind_secs.is_finite() && behind_secs > 0.0 {
            (behind_secs.round() as u64).min(max_behind)
        } else {
            0
        };
        Self {
            cache_key,
            video,
            audio,
            dur_sec,
            max_dvr_segs,
            start_behind_secs,
            head: Mutex::new(HeadTracker {
                head_sq,
                observed_at: Instant::now(),
            }),
            first_served: Mutex::new(None),
            fatal_error: AtomicBool::new(false),
            consecutive_failures: AtomicU32::new(0),
        }
    }

    fn mark_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    fn mark_fatal(&self) {
        self.fatal_error.store(true, Ordering::Release);
    }

    fn record_failure(&self, hard_fatal: bool) {
        if hard_fatal || self.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1 >= 3 {
            self.mark_fatal();
        }
    }

    fn has_fatal_error(&self) -> bool {
        self.fatal_error.load(Ordering::Acquire)
    }

    fn observe_head(&self, new_head: u64) {
        let mut h = self.head.lock();
        if new_head >= h.head_sq || h.observed_at.elapsed() > Duration::from_secs(10) {
            h.head_sq = new_head;
            h.observed_at = Instant::now();
            remember_head(&self.cache_key, new_head);
        }
    }

    fn latest_head(&self) -> u64 {
        let h = self.head.lock();
        let elapsed_segs = h.observed_at.elapsed().as_secs() / self.dur_sec;
        h.head_sq.saturating_add(elapsed_segs)
    }

    fn dvr_window_secs(&self) -> f64 {
        if self.max_dvr_segs == 0 {
            return 0.0;
        }
        let segs = self.latest_head().min(self.max_dvr_segs);
        (segs * self.dur_sec) as f64
    }

    fn current_window(&self) -> (u64, u64) {
        let est_head = self.latest_head();
        let win_segs = self.max_dvr_segs.max(12).min(est_head);
        let start_num = est_head.saturating_sub(win_segs);
        (start_num, est_head)
    }

    fn record_first_served(&self, is_video: bool, sq: u64) {
        if !is_video && self.video.is_some() {
            return;
        }
        let mut first = self.first_served.lock();
        if first.is_none() {
            *first = Some((sq, self.latest_head()));
        }
    }

    fn actual_start_behind_secs(&self) -> Option<f64> {
        let (sq, head) = (*self.first_served.lock())?;
        Some((head.saturating_sub(sq) * self.dur_sec) as f64)
    }
}

/// Handle to an active loopback live DASH playback session for DVR tracking.
#[derive(Clone)]
pub struct LiveDashHandle {
    session: Arc<LiveSession>,
    pub url: String,
}

impl LiveDashHandle {
    /// Total seekable DVR window in seconds (0 when DVR is disabled).
    pub fn dvr_window_secs(&self) -> f64 {
        self.session.dvr_window_secs()
    }

    /// Actual behind-live offset (seconds) of the first decoded segment, once fetched.
    pub fn actual_start_behind_secs(&self) -> Option<f64> {
        self.session.actual_start_behind_secs()
    }

    /// Latest estimated live-edge sequence number for zero-latency reloads/seeks.
    pub fn latest_head(&self) -> u64 {
        self.session.latest_head()
    }

    /// Whether upstream media requests hit a fatal rejection (e.g. HTTP 403 / repeated failures).
    pub fn has_fatal_error(&self) -> bool {
        self.session.has_fatal_error()
    }
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Builds a dynamic MPEG-DASH MPD (`isoff-live:2011`) for a live video/audio pair.
/// `start_behind_secs` sets `suggestedPresentationDelay` so FFmpeg's `dash`
/// demuxer starts directly at the requested DVR point while keeping `max_seq_no`
/// at the real live edge (`head_sq`).
pub fn build_mpd(
    video: Option<&TrackSpec>,
    audio: Option<&TrackSpec>,
    dur_sec: u64,
    start_num: u64,
    head_sq: u64,
    start_behind_secs: u64,
    now_utc: chrono::DateTime<chrono::Utc>,
) -> String {
    let dur_sec = dur_sec.clamp(1, 10);
    let dur_ms = dur_sec * 1000;
    let segs_available = head_sq.saturating_sub(start_num);
    let avail_secs = segs_available * dur_sec;
    let avail_start = now_utc - chrono::Duration::seconds(avail_secs as i64);
    let avail_iso = avail_start.format("%Y-%m-%dT%H:%M:%SZ");
    let shift_secs = segs_available.max(1) * dur_sec;
    // Never let suggestedPresentationDelay exceed avail_secs: FFmpeg's dashdec
    // computes (now - availabilityStartTime - suggestedPresentationDelay) / dur
    // and wraps into bogus huge sequence numbers on brand-new streams (head_sq 0/1)
    // or left-edge seeks if delay > avail_secs. Use base_delay as a lower bound
    // rather than adding it onto start_behind_secs so DVR seeks don't drift back.
    let suggested_delay = if avail_secs == 0 {
        0
    } else {
        let base_delay = (dur_sec * 2).max(3).min(avail_secs);
        start_behind_secs.max(base_delay).min(avail_secs)
    };

    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" \
profiles=\"urn:mpeg:dash:profile:isoff-live:2011\" \
type=\"dynamic\" \
availabilityStartTime=\"{avail_iso}\" \
minimumUpdatePeriod=\"PT{dur_sec}S\" \
minBufferTime=\"PT{dur_sec}S\" \
timeShiftBufferDepth=\"PT{shift_secs}S\" \
suggestedPresentationDelay=\"PT{suggested_delay}S\">\n\
  <Period id=\"0\" start=\"PT0S\">\n"
    );

    if let Some(v) = video {
        let mime = xml_escape(&v.mime);
        let codecs = xml_escape(&v.codecs);
        let bw = v.bitrate.max(1_000_000);
        let fps = v.fps.max(1);
        let ext = v.ext();
        let id = if v.itag > 0 { v.itag } else { 1 };
        xml.push_str(&format!(
            "    <AdaptationSet id=\"0\" mimeType=\"{mime}\" contentType=\"video\" segmentAlignment=\"true\" startWithSAP=\"1\">\n\
      <Representation id=\"{id}\" codecs=\"{codecs}\" width=\"{}\" height=\"{}\" frameRate=\"{fps}\" bandwidth=\"{bw}\">\n\
        <SegmentTemplate timescale=\"1000\" duration=\"{dur_ms}\" startNumber=\"{start_num}\" media=\"v/$Number$.{ext}\"/>\n\
      </Representation>\n\
    </AdaptationSet>\n",
            v.width, v.height
        ));
    }

    if let Some(a) = audio {
        let mime = xml_escape(&a.mime);
        let codecs = xml_escape(&a.codecs);
        let bw = a.bitrate.max(64_000);
        let sr = a.sample_rate.max(8_000);
        let ext = a.ext();
        let id = if a.itag > 0 { a.itag } else { 2 };
        xml.push_str(&format!(
            "    <AdaptationSet id=\"1\" mimeType=\"{mime}\" contentType=\"audio\" segmentAlignment=\"true\" startWithSAP=\"1\">\n\
      <Representation id=\"{id}\" codecs=\"{codecs}\" audioSamplingRate=\"{sr}\" bandwidth=\"{bw}\">\n\
        <SegmentTemplate timescale=\"1000\" duration=\"{dur_ms}\" startNumber=\"{start_num}\" media=\"a/$Number$.{ext}\"/>\n\
      </Representation>\n\
    </AdaptationSet>\n"
        ));
    }

    xml.push_str("  </Period>\n</MPD>\n");
    xml
}

struct LiveDashServer {
    port: u16,
    token: String,
    sessions: Mutex<HashMap<u64, Arc<LiveSession>>>,
    next_id: Mutex<u64>,
}

impl LiveDashServer {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, self.token)
    }

    fn register(&self, session: Arc<LiveSession>) -> u64 {
        let id = {
            let mut n = self.next_id.lock();
            *n += 1;
            *n
        };
        let mut map = self.sessions.lock();
        if map.len() > 64 {
            let min_keep = id.saturating_sub(32);
            map.retain(|k, _| *k >= min_keep);
        }
        map.insert(id, session);
        id
    }

    fn get(&self, id: u64) -> Option<Arc<LiveSession>> {
        self.sessions.lock().get(&id).cloned()
    }
}

fn server() -> Option<&'static LiveDashServer> {
    static SRV: OnceLock<Option<LiveDashServer>> = OnceLock::new();
    SRV.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        let token = {
            use rand::Rng;
            let mut rng = rand::rng();
            (0..24).map(|_| format!("{:02x}", rng.random::<u8>())).collect()
        };
        std::thread::Builder::new()
            .name("live-dash-proxy".into())
            .spawn(move || {
                for conn in listener.incoming().flatten() {
                    std::thread::spawn(move || {
                        let _ = serve_conn(conn);
                    });
                }
            })
            .ok()?;
        Some(LiveDashServer {
            port,
            token,
            sessions: Mutex::new(HashMap::new()),
            next_id: Mutex::new(0),
        })
    })
    .as_ref()
}

fn strip_token<'a>(path: &'a str, token: &str) -> Option<&'a str> {
    let rest = path.strip_prefix('/')?;
    let given = rest.get(..token.len())?;
    let same = given.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
    let rest = &rest[token.len()..];
    (same && rest.starts_with('/')).then_some(rest)
}

/// Non-blocking registration of a live DASH playback session starting `behind_secs`
/// behind the live edge using only `head_hint` or the short-lived `HEAD_CACHE`.
/// Returns `None` on cache miss so UI callers can run `probe_head` off the UI thread.
pub fn open_at_cached(
    video: Option<&StreamFormat>,
    audio: Option<&StreamFormat>,
    behind_secs: f64,
    head_hint: Option<u64>,
) -> Option<LiveDashHandle> {
    let primary = video.or(audio)?;
    let meta = format_meta(&primary.url);
    let dur_sec = meta.target_duration_secs.clamp(1, 10);
    let head_sq = head_hint.or_else(|| cached_head(&primary.url, dur_sec))?;
    open_with_head(video, audio, behind_secs, head_sq, meta)
}

/// Registers a live DASH playback session starting `behind_secs` behind the live
/// edge (`0.0` = live edge), falling back to a network `probe_head` on cache miss.
#[cfg(test)]
pub fn open_at(
    video: Option<&StreamFormat>,
    audio: Option<&StreamFormat>,
    behind_secs: f64,
    head_hint: Option<u64>,
) -> Option<LiveDashHandle> {
    let primary = video.or(audio)?;
    let meta = format_meta(&primary.url);
    let dur_sec = meta.target_duration_secs.clamp(1, 10);
    let head_sq = head_hint
        .or_else(|| cached_head(&primary.url, dur_sec))
        .or_else(|| probe_head(&primary.url))?;
    open_with_head(video, audio, behind_secs, head_sq, meta)
}

fn open_with_head(
    video: Option<&StreamFormat>,
    audio: Option<&StreamFormat>,
    behind_secs: f64,
    head_sq: u64,
    meta: LiveFormatMeta,
) -> Option<LiveDashHandle> {
    let primary = video.or(audio)?;
    let srv = server()?;
    let session = Arc::new(LiveSession::new(
        live_cache_key(&primary.url),
        video.map(TrackSpec::from_format),
        audio.map(TrackSpec::from_format),
        head_sq,
        meta,
        behind_secs,
    ));
    let id = srv.register(session.clone());
    Some(LiveDashHandle {
        session,
        url: format!("{}/ld/{id}/live.mpd", srv.base()),
    })
}

/// Registers a live DASH playback session at the live edge and returns its loopback MPD URL.
#[cfg(test)]
pub fn local_dash_url(video: Option<&StreamFormat>, audio: Option<&StreamFormat>) -> Option<String> {
    open_at(video, audio, 0.0, None).map(|h| h.url)
}

fn read_bounded_line<R: BufRead>(reader: &mut R, max_len: usize) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let mut limited = reader.take(max_len as u64 + 1);
    let n = limited.read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > max_len {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "line too long"));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

fn serve_conn(mut conn: TcpStream) -> std::io::Result<()> {
    conn.set_nodelay(true).ok();
    conn.set_read_timeout(Some(Duration::from_secs(10))).ok();
    conn.set_write_timeout(Some(Duration::from_secs(15))).ok();
    let mut reader = BufReader::new(conn.try_clone()?);
    let Some(req_line) = read_bounded_line(&mut reader, 4096)? else {
        return Ok(());
    };
    let mut range_hdr: Option<String> = None;
    for _ in 0..64 {
        let Some(line) = read_bounded_line(&mut reader, 4096)? else {
            break;
        };
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some((name, val)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("range") {
                let v = val.trim();
                if !v.is_empty() {
                    range_hdr = Some(v.to_string());
                }
            }
        }
    }
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    if method != "GET" && method != "HEAD" {
        return conn.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let raw_path = parts.next().unwrap_or("");
    let path_only = raw_path.split(['?', '#']).next().unwrap_or(raw_path);
    let Some(srv) = server() else { return Ok(()) };
    let Some(path) = strip_token(path_only, &srv.token) else {
        return conn.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let Some(rest) = path.strip_prefix("/ld/") else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let Some((id_str, tail)) = rest.split_once('/') else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let Ok(id) = id_str.parse::<u64>() else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let Some(session) = srv.get(id) else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };

    if tail == "live.mpd" {
        let (start_num, head_sq) = session.current_window();
        let mpd = build_mpd(
            session.video.as_ref(),
            session.audio.as_ref(),
            session.dur_sec,
            start_num,
            head_sq,
            session.start_behind_secs,
            chrono::Utc::now(),
        );
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/dash+xml\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            mpd.len()
        );
        conn.write_all(head.as_bytes())?;
        if method != "HEAD" {
            conn.write_all(mpd.as_bytes())?;
        }
        return conn.flush();
    }

    let Some((kind, seg_file)) = tail.split_once('/') else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let Some((sq_str, ext)) = seg_file.split_once('.') else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let Ok(sq) = sq_str.parse::<u64>() else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let (track, is_video) = match kind {
        "v" => (session.video.as_ref(), true),
        "a" => (session.audio.as_ref(), false),
        _ => (None, false),
    };
    let Some(track) = track else {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    if ext != track.ext() {
        return conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    serve_segment(&mut conn, &session, track, is_video, sq, range_hdr.as_deref(), method == "HEAD")
}

fn open_segment(
    session: &LiveSession,
    track: &TrackSpec,
    sq: u64,
    range: Option<&str>,
    head_only: bool,
) -> Option<ureq::http::Response<ureq::Body>> {
    let url = append_query(&track.url, &format!("sq={sq}"));
    let ag = agent();
    let mut last_resp = None;
    for attempt in 0..4u64 {
        let mut req = if head_only { ag.head(&url) } else { ag.get(&url) };
        req = req.header("User-Agent", http::BROWSER_UA);
        if let Some(r) = range {
            req = req.header("Range", r);
        }
        match req.call() {
            Ok(resp) => {
                if let Some(h) = parse_head_seqnum(&resp) {
                    session.observe_head(h);
                }
                let st = resp.status().as_u16();
                if st < 400 {
                    session.mark_success();
                    return Some(resp);
                }
                if st == 403 {
                    session.record_failure(true);
                    return Some(resp);
                }
                if st == 404 && attempt < 3 {
                    let head = session.latest_head();
                    // Segment right at or just ahead of the live edge: wait briefly for YouTube to publish it.
                    if sq >= head && sq <= head.saturating_add(2) {
                        std::thread::sleep(Duration::from_millis(700));
                        last_resp = Some(resp);
                        continue;
                    }
                }
                if st >= 500 && attempt < 2 {
                    std::thread::sleep(Duration::from_millis(300 * (attempt + 1)));
                    last_resp = Some(resp);
                    continue;
                }
                let head = session.latest_head();
                if st >= 500 || (st == 404 && sq < head) {
                    session.record_failure(false);
                }
                return Some(resp);
            }
            Err(_) if attempt < 2 => {
                std::thread::sleep(Duration::from_millis(300 * (attempt + 1)));
            }
            Err(_) => {
                session.record_failure(false);
                break;
            }
        }
    }
    last_resp
}

fn serve_segment(
    conn: &mut TcpStream,
    session: &LiveSession,
    track: &TrackSpec,
    is_video: bool,
    sq: u64,
    range: Option<&str>,
    head_only: bool,
) -> std::io::Result<()> {
    if session.has_fatal_error() {
        return conn.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let Some(resp) = open_segment(session, track, sq, range, head_only) else {
        return conn.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let status = resp.status().as_u16();
    if status < 400 && !head_only {
        session.record_first_served(is_video, sq);
    }
    let reason = match status {
        200 => "OK",
        206 => "Partial Content",
        403 => "Forbidden",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        _ if status >= 500 => "Bad Gateway",
        _ => "Error",
    };
    let hdr = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let ctype = hdr("content-type").unwrap_or_else(|| track.mime.clone());
    let length = hdr("content-length");
    let can_have_body = status >= 200 && status != 204 && status != 304 && !head_only;
    let chunked = length.is_none() && can_have_body;
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nConnection: close\r\n");
    if let Some(l) = &length {
        head.push_str(&format!("Content-Length: {l}\r\n"));
    } else if chunked {
        head.push_str("Transfer-Encoding: chunked\r\n");
    } else if !can_have_body {
        head.push_str("Content-Length: 0\r\n");
    }
    if let Some(cr) = hdr("content-range") {
        head.push_str(&format!("Content-Range: {cr}\r\n"));
    }
    head.push_str("\r\n");
    conn.write_all(head.as_bytes())?;
    if !can_have_body {
        return conn.flush();
    }
    let mut body = resp.into_body().into_reader();
    let mut buf = [0u8; 32 * 1024];
    let mut writer = BufWriter::with_capacity(buf.len() + 32, conn);
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if chunked {
            write!(writer, "{n:x}\r\n")?;
            writer.write_all(&buf[..n])?;
            writer.write_all(b"\r\n")?;
        } else {
            writer.write_all(&buf[..n])?;
        }
        writer.flush()?;
    }
    if chunked {
        writer.write_all(b"0\r\n\r\n")?;
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn detects_live_dash_urls() {
        assert!(is_live_dash_url(
            "https://rr1---sn-abc.googlevideo.com/videoplayback?expire=1&source=yt_live_broadcast&live=1&noclen=1&itag=308"
        ));
        assert!(is_live_dash_url(
            "https://rr1---sn-abc.googlevideo.com/videoplayback?source=yt_live_broadcast&noclen=1&itag=140"
        ));
        assert!(!is_live_dash_url(
            "https://rr1---sn-abc.googlevideo.com/videoplayback?expire=1&itag=308&clen=12345"
        ));
        assert!(!is_live_dash_url("https://example.invalid/v?live=1&noclen=1"));
        assert!(!is_live_dash_url("not-a-url"));
    }

    #[test]
    fn builds_valid_dynamic_mpd_for_vp9_1440p60_and_av1_4k() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-27T14:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let v = TrackSpec {
            itag: 308,
            url: "https://rr1.googlevideo.com/videoplayback?live=1&noclen=1".into(),
            mime: "video/webm".into(),
            codecs: "vp9".into(),
            width: 2560,
            height: 1440,
            fps: 60,
            bitrate: 9_016_000,
            sample_rate: 44100,
        };
        let a = TrackSpec {
            itag: 140,
            url: "https://rr1.googlevideo.com/videoplayback?live=1&noclen=1".into(),
            mime: "audio/mp4".into(),
            codecs: "mp4a.40.2".into(),
            width: 0,
            height: 0,
            fps: 0,
            bitrate: 144_000,
            sample_rate: 44100,
        };
        let mpd = build_mpd(Some(&v), Some(&a), 5, 2750, 2762, 0, now);
        assert!(mpd.contains("profiles=\"urn:mpeg:dash:profile:isoff-live:2011\""));
        assert!(mpd.contains("type=\"dynamic\""));
        assert!(mpd.contains("availabilityStartTime=\"2026-11-27T13:59:00Z\""));
        assert!(mpd.contains("suggestedPresentationDelay=\"PT10S\""));
        assert!(mpd.contains("width=\"2560\" height=\"1440\" frameRate=\"60\""));
        assert!(mpd.contains("media=\"v/$Number$.webm\""));
        assert!(mpd.contains("media=\"a/$Number$.m4s\""));
        assert!(mpd.contains("startNumber=\"2750\""));
        assert!(mpd.contains("duration=\"5000\""));

        // Rewound -13:04 (784s) keeps real availabilityStartTime and sets exact suggestedPresentationDelay (no +10s drift)
        let dvr = build_mpd(Some(&v), Some(&a), 5, 1000, 2000, 784, now);
        assert!(dvr.contains("timeShiftBufferDepth=\"PT5000S\""));
        assert!(dvr.contains("suggestedPresentationDelay=\"PT784S\""));

        // Rewind past the DVR window clamps inside avail_secs (never exceeds availabilityStartTime span)
        let clamped = build_mpd(Some(&v), Some(&a), 5, 2750, 2762, 9999, now);
        assert!(clamped.contains("suggestedPresentationDelay=\"PT60S\""));

        // Short / newly started streams (head_sq 0..=12): suggestedPresentationDelay must NEVER exceed avail_secs
        for head_sq in 0..=12u64 {
            for behind in [0u64, 5, 15, 60, 120] {
                let xml = build_mpd(Some(&v), Some(&a), 5, 0, head_sq, behind, now);
                let avail_secs = head_sq * 5;
                let tag = "suggestedPresentationDelay=\"PT";
                let rest = xml.split(tag).nth(1).expect("suggestedPresentationDelay");
                let delay: u64 = rest.split('S').next().unwrap().parse().unwrap();
                assert!(
                    delay <= avail_secs,
                    "head_sq={head_sq} behind={behind}: delay={delay} > avail_secs={avail_secs}"
                );
                if head_sq == 0 {
                    assert_eq!(delay, 0);
                }
            }
        }

        // Audio-only MPD omits video AdaptationSet
        let audio_only = build_mpd(None, Some(&a), 2, 100, 110, 0, now);
        assert!(!audio_only.contains("contentType=\"video\""));
        assert!(audio_only.contains("contentType=\"audio\""));
        assert!(audio_only.contains("duration=\"2000\""));
    }

    #[test]
    fn open_at_cached_is_non_blocking_on_miss_and_reuses_hint() {
        let cold = StreamFormat {
            itag: 308,
            url: "http://127.0.0.1:9/videoplayback?id=coldmiss.1&live=1&noclen=1&itag=308".into(),
            mime: "video/webm".into(),
            codecs: "vp9".into(),
            width: 2560,
            height: 1440,
            fps: 60,
            ..Default::default()
        };
        assert!(open_at_cached(Some(&cold), None, 0.0, None).is_none());
        let h = open_at_cached(Some(&cold), None, 0.0, Some(42)).expect("uses head_hint");
        assert_eq!(h.latest_head(), 42);
    }

    #[test]
    fn upstream_403_marks_session_fatal_and_fast_fails() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let up_addr = upstream.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_bg = hits.clone();
        std::thread::spawn(move || {
            for mut stream in upstream.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 {
                    continue;
                }
                hits_bg.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            }
        });
        let video = StreamFormat {
            itag: 308,
            url: format!("http://{up_addr}/videoplayback?id=fatal403.1&live=1&noclen=1"),
            mime: "video/webm".into(),
            codecs: "vp9".into(),
            width: 2560,
            height: 1440,
            fps: 60,
            ..Default::default()
        };
        let handle = open_at_cached(Some(&video), None, 0.0, Some(100)).expect("open cached");
        assert!(!handle.has_fatal_error());
        let seg1 = handle.url.replace("live.mpd", "v/98.webm");
        let r1 = agent().get(&seg1).call().unwrap();
        assert_eq!(r1.status().as_u16(), 403);
        assert!(handle.has_fatal_error());
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // Second request fast-fails locally without hitting upstream again
        let seg2 = handle.url.replace("live.mpd", "v/99.webm");
        let r2 = agent().get(&seg2).call().unwrap();
        assert_eq!(r2.status().as_u16(), 403);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // Invalid extension and method are rejected cleanly
        let bad_ext = handle.url.replace("live.mpd", "v/99.txt");
        assert_eq!(agent().get(&bad_ext).call().unwrap().status().as_u16(), 404);
        assert_eq!(agent().post(&handle.url).send_empty().unwrap().status().as_u16(), 405);
    }

    #[test]
    fn open_at_computes_dvr_window_and_tracks_first_served_behind_offset() {
        let video = StreamFormat {
            itag: 308,
            url: "https://rr1.googlevideo.com/videoplayback?id=dvrtest.1&live=1&noclen=1&itag=308".into(),
            mime: "video/webm".into(),
            codecs: "vp9".into(),
            bitrate: 9_000_000,
            width: 2560,
            height: 1440,
            fps: 60,
            ..Default::default()
        };
        remember_format_meta(
            &video.url,
            LiveFormatMeta { target_duration_secs: 5, max_dvr_secs: 43200, audio_sample_rate: 44100 },
        );
        // 1200 segments * 5s = 6000s broadcast (< 4h cap)
        let handle = open_at(Some(&video), None, 784.0, Some(1200)).expect("open_at");
        assert_eq!(handle.dvr_window_secs(), 6000.0);
        assert_eq!(handle.latest_head(), 1200);
        assert_eq!(handle.actual_start_behind_secs(), None);
        handle.session.record_first_served(true, 1043);
        assert_eq!(handle.actual_start_behind_secs(), Some(785.0));

        // Non-DVR broadcast reports 0s DVR window
        let no_dvr = StreamFormat {
            url: "https://rr1.googlevideo.com/videoplayback?id=nodvr.1&live=1&noclen=1&itag=308".into(),
            ..video
        };
        remember_format_meta(
            &no_dvr.url,
            LiveFormatMeta { target_duration_secs: 5, max_dvr_secs: 0, audio_sample_rate: 44100 },
        );
        let h2 = open_at(Some(&no_dvr), None, 500.0, Some(1200)).expect("open_at non-dvr");
        assert_eq!(h2.dvr_window_secs(), 0.0);
    }

    #[test]
    fn loopback_server_enforces_token_serves_mpd_and_retries_live_edge_404() {
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let up_addr = upstream.local_addr().unwrap();
        let seg_hits = Arc::new(AtomicUsize::new(0));
        let hits_bg = seg_hits.clone();

        std::thread::spawn(move || {
            for stream in upstream.incoming().flatten() {
                let mut stream = stream;
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut req_line = String::new();
                if reader.read_line(&mut req_line).unwrap_or(0) == 0 {
                    continue;
                }
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" || line == "\n" {
                        break;
                    }
                }
                if req_line.starts_with("HEAD ") {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nX-Head-Seqnum: 500\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                } else if req_line.contains("sq=501") {
                    let n = hits_bg.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        // First request for sq=501 is slightly early (head=500) -> 404
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nX-Head-Seqnum: 500\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    } else {
                        // Retry succeeds as YouTube publishes sq=501
                        let _ = stream.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: video/webm\r\nX-Head-Seqnum: 501\r\nContent-Length: 10\r\nConnection: close\r\n\r\nWEBM_CHUNK",
                        );
                    }
                } else {
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: video/webm\r\nX-Head-Seqnum: 500\r\nContent-Length: 4\r\nConnection: close\r\n\r\nDATA",
                    );
                }
            }
        });

        let video = StreamFormat {
            itag: 308,
            url: format!("http://{up_addr}/videoplayback?live=1&noclen=1"),
            mime: "video/webm".into(),
            codecs: "vp9".into(),
            bitrate: 9_000_000,
            width: 2560,
            height: 1440,
            fps: 60,
            ..Default::default()
        };
        remember_format_meta(&video.url, LiveFormatMeta { target_duration_secs: 5, max_dvr_secs: 3600, audio_sample_rate: 44100 });
        let mpd_url = local_dash_url(Some(&video), None).expect("local dash url");

        // Wrong token must be refused with 403
        let bad_url = {
            let parsed = url::Url::parse(&mpd_url).unwrap();
            format!("http://127.0.0.1:{}/wrongtoken/ld/1/live.mpd", parsed.port().unwrap())
        };
        let bad = agent().get(&bad_url).call().unwrap();
        assert_eq!(bad.status().as_u16(), 403);

        // MPD served cleanly with 1440p60 VP9
        let mut r_mpd = agent().get(&mpd_url).call().unwrap();
        assert_eq!(r_mpd.status().as_u16(), 200);
        let mpd_text = r_mpd.body_mut().read_to_string().unwrap();
        assert!(mpd_text.contains("width=\"2560\" height=\"1440\""));
        assert!(mpd_text.contains("media=\"v/$Number$.webm\""));

        // Early live-edge segment sq=501 retries 404 -> 200 and streams body
        let seg_url = mpd_url.replace("live.mpd", "v/501.webm");
        let mut r_seg = agent().get(&seg_url).call().unwrap();
        assert_eq!(r_seg.status().as_u16(), 200);
        let bytes = r_seg.body_mut().read_to_vec().unwrap();
        assert_eq!(bytes, b"WEBM_CHUNK");
        assert!(seg_hits.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    #[ignore = "network + libmpv: verifies real 1440p/4K live playback end-to-end"]
    fn live_1440p_end_to_end_with_libmpv() {
        crate::store::init();
        let vid = std::env::var("VID").unwrap_or_else(|_| "Fp5ZSV6cbQI".into());
        let info = crate::api::streams::get_streams(&vid).expect("resolve live stream");
        println!(
            "resolved {} via {} is_live={} hls_only={} hls={} max_h={:?} heights={:?}",
            vid,
            info.source,
            info.is_live,
            info.hls_only,
            info.hls_url.is_some(),
            info.video.iter().map(|f| f.height).max(),
            info.video.iter().map(|f| (f.itag, f.height, f.fps, f.codecs.as_str())).take(8).collect::<Vec<_>>()
        );
        assert!(info.is_live, "stream must be live");
        assert!(info.video.iter().any(|f| f.height >= 1440), "must expose >=1440p video formats");
        assert!(info.hls_url.is_some(), "must preserve visionOS hls_url for preview/fallback");

        let want_q = std::env::var("Q").ok().and_then(|s| s.parse().ok()).unwrap_or(1440u32);
        let sel = crate::api::streams::playback_stream(&info, want_q, "vp9", None, false, false)
            .expect("playback selection");
        let (v, a) = match sel {
            crate::api::streams::PlaybackStream::Adaptive(v, a) => (v, a),
            _ => panic!("expected Adaptive live DASH stream for >=1440p live"),
        };
        let v = v.expect("video format");
        assert!(v.height >= 1440, "picked video height must be >=1440, got {}", v.height);
        let mpd_url = open_at(Some(v), a, 0.0, None).expect("live dash open").url;
        println!("mpd_url={mpd_url} (picked itag={} {}x{}@{} {})", v.itag, v.width, v.height, v.fps, v.codecs);

        // Verify MPD and real libmpv decoding when LIBMPV_PATH or third_party/mpv/libmpv-2.dll is available
        #[cfg(windows)]
        {
            let dll = std::env::var("LIBMPV_PATH")
                .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/third_party/mpv/libmpv-2.dll").to_string());
            if std::path::Path::new(&dll).is_file() {
                use std::ffi::{CStr, CString, c_char, c_int, c_void};
                unsafe {
                    let lib = libloading::Library::new(&dll).expect("load libmpv-2.dll");
                    let create: libloading::Symbol<unsafe extern "C" fn() -> *mut c_void> = lib.get(b"mpv_create\0").unwrap();
                    let init: libloading::Symbol<unsafe extern "C" fn(*mut c_void) -> c_int> = lib.get(b"mpv_initialize\0").unwrap();
                    let set_opt: libloading::Symbol<unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int> =
                        lib.get(b"mpv_set_option_string\0").unwrap();
                    let cmd: libloading::Symbol<unsafe extern "C" fn(*mut c_void, *mut *const c_char) -> c_int> =
                        lib.get(b"mpv_command\0").unwrap();
                    let get_str: libloading::Symbol<unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_char> =
                        lib.get(b"mpv_get_property_string\0").unwrap();
                    let free: libloading::Symbol<unsafe extern "C" fn(*mut c_void)> = lib.get(b"mpv_free\0").unwrap();
                    let destroy: libloading::Symbol<unsafe extern "C" fn(*mut c_void)> =
                        lib.get(b"mpv_terminate_destroy\0").unwrap();

                    let h = create();
                    for (k, val) in [("vo", "null"), ("ao", "null"), ("ytdl", "no"), ("idle", "yes"), ("cache", "yes"), ("network-timeout", "15")] {
                        let ck = CString::new(k).unwrap();
                        let cv = CString::new(val).unwrap();
                        assert_eq!(set_opt(h, ck.as_ptr(), cv.as_ptr()), 0);
                    }
                    if let Ok(lf) = std::env::var("STREAMTUBE_MPV_LOG") {
                        let ck = CString::new("log-file").unwrap();
                        let cv = CString::new(lf).unwrap();
                        assert_eq!(set_opt(h, ck.as_ptr(), cv.as_ptr()), 0);
                    }
                    assert_eq!(init(h), 0);
                    let c0 = CString::new("loadfile").unwrap();
                    let c1 = CString::new(mpd_url.as_str()).unwrap();
                    let c2 = CString::new("replace").unwrap();
                    let mut args = [c0.as_ptr(), c1.as_ptr(), c2.as_ptr(), std::ptr::null()];
                    assert_eq!(cmd(h, args.as_mut_ptr()), 0);

                    let prop = |name: &str| -> Option<String> {
                        let cn = CString::new(name).unwrap();
                        let ptr = get_str(h, cn.as_ptr());
                        if ptr.is_null() {
                            return None;
                        }
                        let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                        free(ptr as *mut c_void);
                        Some(s)
                    };

                    let mut decoded_h = 0u32;
                    let mut pos = 0.0f64;
                    let mut acodec = None;
                    for sec in 1..=14 {
                        std::thread::sleep(Duration::from_secs(1));
                        let w = prop("video-params/w");
                        let hh = prop("video-params/h");
                        let vc = prop("video-codec");
                        let ac = prop("audio-codec-name");
                        let tp = prop("time-pos");
                        let cache = prop("demuxer-cache-duration");
                        if sec % 2 == 0 || hh.is_some() {
                            println!("  t={sec}s: {w:?}x{hh:?} v={vc:?} a={ac:?} pos={tp:?} cache={cache:?}");
                        }
                        if let Some(h_str) = hh.as_deref() {
                            decoded_h = h_str.parse().unwrap_or(0);
                        }
                        if let Some(p_str) = tp.as_deref() {
                            pos = p_str.parse().unwrap_or(0.0);
                        }
                        if ac.is_some() {
                            acodec = ac;
                        }
                        if decoded_h >= 1440 && pos >= 3.0 && acodec.is_some() {
                            break;
                        }
                    }
                    assert!(decoded_h >= 1440, "libmpv must decode >=1440p live video, got {decoded_h}");
                    assert!(pos >= 2.0, "libmpv playback must advance cleanly, got pos={pos}");
                    assert!(acodec.is_some(), "libmpv must decode live audio track");

                    // 2. DVR rewind to -13:00 (780s behind live) at 1440p60 on the SAME mpv handle
                    let head_now = probe_head(&v.url).expect("head");
                    let dvr_1440 = open_at(Some(v), a, 780.0, Some(head_now)).expect("open_at -13m 1440p");
                    let opts = CString::new("demuxer-lavf-analyzeduration=0.5,cache-pause-initial=no,demuxer-readahead-secs=10,pause=no").unwrap();
                    let idx_s = CString::new("-1").unwrap();
                    let c1_dvr = CString::new(dvr_1440.url.as_str()).unwrap();
                    let mut args_dvr = [c0.as_ptr(), c1_dvr.as_ptr(), c2.as_ptr(), idx_s.as_ptr(), opts.as_ptr(), std::ptr::null()];
                    assert_eq!(cmd(h, args_dvr.as_mut_ptr()), 0);
                    let mut dvr_h = 0u32;
                    let mut dvr_pos = 0.0f64;
                    for sec in 1..=10 {
                        std::thread::sleep(Duration::from_secs(1));
                        let hh = prop("video-params/h");
                        let tp = prop("time-pos");
                        if let Some(s) = hh.as_deref() { dvr_h = s.parse().unwrap_or(0); }
                        if let Some(s) = tp.as_deref() { dvr_pos = s.parse().unwrap_or(0.0); }
                        if sec % 2 == 0 || dvr_h >= 1440 {
                            println!("  [DVR -13m 1440p] t={sec}s: h={hh:?} pos={tp:?} behind={:?}", dvr_1440.actual_start_behind_secs());
                        }
                        if dvr_h >= 1440 && dvr_pos >= 2.0 && dvr_1440.actual_start_behind_secs().is_some() {
                            break;
                        }
                    }
                    let actual_behind = dvr_1440.actual_start_behind_secs().expect("first served sq recorded");
                    assert!(dvr_h >= 1440, "DVR -13m must decode >=1440p, got {dvr_h}");
                    assert!((actual_behind - 780.0).abs() <= 25.0, "expected ~780s behind live, got {actual_behind}");

                    // 3. Quality switch at -13:00: 1440p -> 1080p on the SAME mpv handle (no start= hang!)
                    if let Some(crate::api::streams::PlaybackStream::Adaptive(Some(v1080), a1080)) =
                        crate::api::streams::playback_stream(&info, 1080, "vp9", None, false, false)
                    {
                        println!("  [Switch -13m 1080p] picked itag={} {}x{}@{} {} mime={}", v1080.itag, v1080.width, v1080.height, v1080.fps, v1080.codecs, v1080.mime);
                        let dvr_1080 = open_at(Some(v1080), a1080, actual_behind, Some(dvr_1440.latest_head())).expect("open_at -13m 1080p");
                        println!("  [Switch -13m 1080p] url={} dvr_win={}", dvr_1080.url, dvr_1080.dvr_window_secs());
                        let c1_1080 = CString::new(dvr_1080.url.as_str()).unwrap();
                        let mut args_1080 = [c0.as_ptr(), c1_1080.as_ptr(), c2.as_ptr(), idx_s.as_ptr(), opts.as_ptr(), std::ptr::null()];
                        assert_eq!(cmd(h, args_1080.as_mut_ptr()), 0);
                        let mut sw_h = 0u32;
                        let mut sw_pos = 0.0f64;
                        for sec in 1..=10 {
                            std::thread::sleep(Duration::from_secs(1));
                            let hh = prop("video-params/h");
                            let tp = prop("time-pos");
                            if let Some(s) = hh.as_deref() { sw_h = s.parse().unwrap_or(0); }
                            if let Some(s) = tp.as_deref() { sw_pos = s.parse().unwrap_or(0.0); }
                            if sec % 2 == 0 || sw_h == 1080 {
                                println!("  [Switch -13m 1080p] t={sec}s: h={hh:?} pos={tp:?} behind={:?}", dvr_1080.actual_start_behind_secs());
                            }
                            if sw_h == 1080 && sw_pos >= 1.0 && dvr_1080.actual_start_behind_secs().is_some() {
                                break;
                            }
                        }
                        assert_eq!(sw_h, 1080, "quality switch to 1080p must decode 1080p cleanly");
                    }

                    // 4. Go Live back to 1440p live edge (behind=0)
                    let live_again = open_at(Some(v), a, 0.0, Some(dvr_1440.latest_head())).expect("go_live 1440p");
                    let c1_live = CString::new(live_again.url.as_str()).unwrap();
                    let mut args_live = [c0.as_ptr(), c1_live.as_ptr(), c2.as_ptr(), idx_s.as_ptr(), opts.as_ptr(), std::ptr::null()];
                    assert_eq!(cmd(h, args_live.as_mut_ptr()), 0);
                    let mut gl_h = 0u32;
                    for sec in 1..=10 {
                        std::thread::sleep(Duration::from_secs(1));
                        let hh = prop("video-params/h");
                        let tp = prop("time-pos");
                        if let Some(s) = hh.as_deref() { gl_h = s.parse().unwrap_or(0); }
                        if sec % 2 == 0 || gl_h >= 1440 {
                            println!("  [Go Live 1440p] t={sec}s: h={hh:?} pos={tp:?} behind={:?}", live_again.actual_start_behind_secs());
                        }
                        if gl_h >= 1440 && live_again.actual_start_behind_secs().is_some() {
                            break;
                        }
                    }
                    let live_behind = live_again.actual_start_behind_secs().expect("go_live first served");
                    assert!(gl_h >= 1440, "Go Live must decode >=1440p, got {gl_h}");
                    assert!(live_behind <= 20.0, "Go Live must land near live edge, got behind={live_behind}");

                    destroy(h);
                }
            }
        }
    }
}
