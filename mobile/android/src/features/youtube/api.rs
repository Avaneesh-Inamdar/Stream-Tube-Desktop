use crate::entities::{Channel, Video};
use image::{ImageFormat, ImageReader, RgbaImage};
use serde_json::{Deserializer, Value, json};
use std::{collections::HashSet, io::Cursor, time::Duration};
use url::Url;

const ORIGIN: &str = "https://www.youtube.com";
const WEB_VERSION: &str = "2.20251120.01.00";
const CHANNEL_VIDEOS: &str = "EgZ2aWRlb3PyBgQKAjoA";
const BROWSER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const MAX_DATA_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESULTS: usize = 40;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Category {
    #[default]
    All,
    Music,
    Games,
    Tech,
    Live,
}

impl Category {
    fn query(self, russian: bool) -> &'static str {
        match (self, russian) {
            (Self::All, false) => "popular videos this week",
            (Self::All, true) => "популярные видео за неделю",
            (Self::Music, false) => "new music official video",
            (Self::Music, true) => "новая музыка клипы",
            (Self::Games, false) => "gaming new releases gameplay",
            (Self::Games, true) => "новинки игр геймплей",
            (Self::Tech, false) => "technology news",
            (Self::Tech, true) => "новости технологий",
            (Self::Live, false) => "live stream now",
            (Self::Live, true) => "прямой эфир сейчас",
        }
    }
}

pub struct Client {
    http: ureq::Agent,
    russian: bool,
}

impl Client {
    pub fn new(russian: bool) -> Self {
        Self {
            http: agent(Duration::from_secs(8)),
            russian,
        }
    }

    pub fn home(&self, category: Category) -> Option<Vec<Video>> {
        if category == Category::All {
            if let Some(videos) = self
                .innertube("browse", json!({ "browseId": "FEwhat_to_watch" }))
                .and_then(|value| parse_search_results(&value, false, None))
                .filter(|videos| !videos.is_empty())
            {
                return Some(videos);
            }
        }
        self.search(category.query(self.russian), false)
    }

    pub fn shorts(&self) -> Option<Vec<Video>> {
        self.search(
            if self.russian {
                "популярные shorts"
            } else {
                "popular YouTube Shorts"
            },
            true,
        )
    }

    pub fn search(&self, query: &str, shorts: bool) -> Option<Vec<Video>> {
        if let Some(results) = self
            .innertube("search", json!({ "query": query }))
            .and_then(|value| parse_search_results(&value, shorts, None))
            .filter(|videos| !videos.is_empty())
        {
            return Some(results);
        }

        let mut endpoint =
            Url::parse("https://www.youtube.com/results").expect("fixed YouTube origin");
        endpoint
            .query_pairs_mut()
            .append_pair("search_query", query);
        let page = self.page_data(endpoint)?;
        parse_search_results(&page, shorts, None)
    }

    pub fn channel(&self, channel: &Channel) -> Option<Vec<Video>> {
        if !is_channel_id(&channel.id) {
            return None;
        }
        if let Some(videos) = self
            .innertube(
                "browse",
                json!({ "browseId": channel.id, "params": CHANNEL_VIDEOS }),
            )
            .and_then(|value| parse_search_results(&value, false, Some(channel)))
            .filter(|videos| !videos.is_empty())
        {
            return Some(videos);
        }
        let endpoint = Url::parse(&format!("{ORIGIN}/channel/{}/videos", channel.id)).ok()?;
        let page = self.page_data(endpoint)?;
        parse_search_results(&page, false, Some(channel))
    }

    fn locale(&self) -> (&'static str, &'static str) {
        if self.russian {
            ("ru", "RU")
        } else {
            ("en", "US")
        }
    }

    fn page_data(&self, mut endpoint: Url) -> Option<Value> {
        let (language, region) = self.locale();
        endpoint
            .query_pairs_mut()
            .append_pair("hl", language)
            .append_pair("gl", region);
        for _ in 0..3 {
            let mut response = self
                .http
                .get(endpoint.as_str())
                .header("User-Agent", BROWSER_AGENT)
                .call()
                .ok()?;
            if response.status().is_redirection() {
                let next = endpoint
                    .join(response.headers().get("Location")?.to_str().ok()?)
                    .ok()?;
                if !safe_data_origin(&next) {
                    return None;
                }
                endpoint = next;
                continue;
            }
            let html = response
                .body_mut()
                .with_config()
                .limit(MAX_DATA_BYTES)
                .read_to_string()
                .ok()?;
            let (_, data) = html.split_once("ytInitialData = ")?;
            return Deserializer::from_str(data)
                .into_iter::<Value>()
                .next()?
                .ok();
        }
        None
    }

    fn innertube(&self, endpoint: &'static str, mut body: Value) -> Option<Value> {
        let (language, region) = self.locale();
        body["context"] = json!({
            "client": {
                "clientName": "WEB",
                "clientVersion": WEB_VERSION,
                "hl": language,
                "gl": region
            }
        });
        let json = body.to_string();
        let bytes = self
            .http
            .post(&format!(
                "{ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false"
            ))
            .header("User-Agent", BROWSER_AGENT)
            .header("X-YouTube-Client-Name", "1")
            .header("X-YouTube-Client-Version", WEB_VERSION)
            .header("Origin", ORIGIN)
            .header("Content-Type", "application/json")
            .send(json.as_bytes())
            .ok()?
            .body_mut()
            .with_config()
            .limit(MAX_DATA_BYTES)
            .read_to_vec()
            .ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

fn safe_data_origin(url: &Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("www.youtube.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
}

pub fn is_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub fn is_channel_id(id: &str) -> bool {
    id.len() == 24
        && id.starts_with("UC")
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub fn video_id(input: &str) -> Option<String> {
    let url = Url::parse(input)
        .or_else(|_| Url::parse(&format!("https://{input}")))
        .ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }

    let mut path = url.path_segments()?;
    let first = path.next()?;
    let id = match url.host_str()? {
        "youtu.be" => first.to_owned(),
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com" => match first {
            "watch" => url
                .query_pairs()
                .find(|(key, _)| key == "v")?
                .1
                .into_owned(),
            "shorts" | "live" | "embed" => path.next()?.to_owned(),
            _ => return None,
        },
        _ => return None,
    };
    (is_video_id(&id) && path.all(str::is_empty)).then_some(id)
}

fn parse_search_results(
    data: &Value,
    shorts: bool,
    channel: Option<&Channel>,
) -> Option<Vec<Video>> {
    let content = data.get("contents")?;
    let mut videos = Vec::new();
    let mut seen = HashSet::new();
    collect_videos(content, shorts, channel, &mut seen, &mut videos);
    Some(videos)
}

fn collect_videos(
    value: &Value,
    shorts: bool,
    channel: Option<&Channel>,
    seen: &mut HashSet<String>,
    videos: &mut Vec<Video>,
) {
    if videos.len() >= MAX_RESULTS {
        return;
    }
    match value {
        Value::Array(items) => {
            for item in items {
                collect_videos(item, shorts, channel, seen, videos);
            }
        }
        Value::Object(fields) => {
            for (key, child) in fields {
                if videos.len() >= MAX_RESULTS {
                    break;
                }
                let video = match key.as_str() {
                    "videoRenderer" | "gridVideoRenderer" if !shorts => {
                        video_renderer(child, channel)
                    }
                    "lockupViewModel" if !shorts => video_lockup(child, channel),
                    "shortsLockupViewModel" if shorts => short_lockup(child),
                    "videoRenderer"
                    | "gridVideoRenderer"
                    | "lockupViewModel"
                    | "shortsLockupViewModel"
                    | "menu"
                    | "menuButton"
                    | "metadata"
                    | "trackingParams"
                    | "header"
                    | "adSlotRenderer"
                    | "promotedSparklesWebRenderer"
                    | "promotedVideoRenderer"
                    | "searchPyvRenderer"
                    | "displayAdRenderer" => None,
                    _ => {
                        collect_videos(child, shorts, channel, seen, videos);
                        None
                    }
                };
                if let Some(video) = video.filter(|video| seen.insert(video.id.clone())) {
                    videos.push(video);
                }
            }
        }
        _ => {}
    }
}

fn video_renderer(item: &Value, fallback: Option<&Channel>) -> Option<Video> {
    let id = item.get("videoId")?.as_str()?.to_owned();
    let title = text(item.get("title"));
    if !is_video_id(&id) || title.is_empty() || item.get("promotedSparklesWebRenderer").is_some() {
        return None;
    }
    let owner = item
        .get("ownerText")
        .or_else(|| item.get("shortBylineText"))
        .or_else(|| item.get("longBylineText"));
    let channel = if let Some(owner) = owner {
        Channel {
            id: field(owner, "/runs/0/navigationEndpoint/browseEndpoint/browseId")
                .filter(|id| is_channel_id(id))
                .unwrap_or_default()
                .to_owned(),
            name: text(Some(owner)),
        }
    } else {
        fallback.cloned().unwrap_or_default()
    };
    Some(Video {
        id,
        title,
        channel,
        duration: text(item.get("lengthText")),
        views: text(
            item.get("shortViewCountText")
                .or_else(|| item.get("viewCountText")),
        ),
        published: text(item.get("publishedTimeText")),
        short: false,
    })
}

fn video_lockup(item: &Value, fallback: Option<&Channel>) -> Option<Video> {
    if item.get("contentType")?.as_str()? != "LOCKUP_CONTENT_TYPE_VIDEO" {
        return None;
    }
    let id = item.get("contentId")?.as_str()?.to_owned();
    let metadata = item.pointer("/metadata/lockupMetadataViewModel")?;
    let title = field(metadata, "/title/content")?.to_owned();
    if !is_video_id(&id) || title.is_empty() {
        return None;
    }
    let rows = metadata.pointer("/metadata/contentMetadataViewModel/metadataRows");
    let owner = rows.and_then(|rows| rows.pointer("/0/metadataParts/0/text"));
    let channel = if let Some(channel) = fallback {
        channel.clone()
    } else {
        Channel {
            id: owner
                .and_then(|node| {
                    field(
                        node,
                        "/commandRuns/0/onTap/innertubeCommand/browseEndpoint/browseId",
                    )
                })
                .filter(|id| is_channel_id(id))
                .unwrap_or_default()
                .to_owned(),
            name: owner
                .and_then(|node| field(node, "/content"))
                .unwrap_or_default()
                .to_owned(),
        }
    };
    let mut video = Video {
        id,
        title,
        channel,
        ..Video::default()
    };
    if let Some(rows) = rows {
        let row = if fallback.is_some() { 0 } else { 1 };
        video.views = field(rows, &format!("/{row}/metadataParts/0/text/content"))
            .unwrap_or_default()
            .to_owned();
        video.published = field(rows, &format!("/{row}/metadataParts/1/text/content"))
            .unwrap_or_default()
            .to_owned();
    }
    video.duration = field(item, "/contentImage/thumbnailViewModel/overlays/0/thumbnailBottomOverlayViewModel/badges/0/thumbnailBadgeViewModel/text").unwrap_or_default().to_owned();
    Some(video)
}

fn short_lockup(item: &Value) -> Option<Video> {
    let id = field(item, "/onTap/innertubeCommand/reelWatchEndpoint/videoId")?.to_owned();
    let title = field(item, "/overlayMetadata/primaryText/content")?.to_owned();
    if !is_video_id(&id) || title.is_empty() {
        return None;
    }
    Some(Video {
        id,
        title,
        short: true,
        views: field(item, "/overlayMetadata/secondaryText/content")
            .unwrap_or_default()
            .to_owned(),
        ..Video::default()
    })
}

fn field<'a>(item: &'a Value, pointer: &str) -> Option<&'a str> {
    item.pointer(pointer)?.as_str()
}

fn text(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(simple) = value.get("simpleText").and_then(Value::as_str) {
        return simple.to_owned();
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .collect()
}

pub fn thumbnail_agent() -> ureq::Agent {
    agent(Duration::from_secs(3))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .max_redirects_will_error(false)
        .build()
        .into()
}

pub fn thumbnail(http: &ureq::Agent, id: &str) -> Option<RgbaImage> {
    if !is_video_id(id) {
        return None;
    }
    let bytes = http
        .get(&format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"))
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(256 * 1024)
        .read_to_vec()
        .ok()?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(480);
    limits.max_image_height = Some(360);
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    Some(reader.decode().ok()?.into_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_youtube_links_and_rejects_untrusted_origins() {
        for input in [
            "https://www.youtube.com/watch?feature=share&v=M7lc1UVf-VE",
            "https://youtu.be/M7lc1UVf-VE?t=30",
            "m.youtube.com/shorts/M7lc1UVf-VE",
            "https://www.youtube.com/live/M7lc1UVf-VE",
        ] {
            assert_eq!(video_id(input).as_deref(), Some("M7lc1UVf-VE"));
        }
        for input in [
            "https://youtube.com.example.org/watch?v=M7lc1UVf-VE",
            "https://www.youtube.com@evil.test/watch?v=M7lc1UVf-VE",
            "https://attacker@youtube.com/watch?v=M7lc1UVf-VE",
            "https://youtu.be/M7lc1UVf-VE/next",
            "https://youtu.be:8443/M7lc1UVf-VE",
            "https://youtu.be/short",
            "http://youtu.be/M7lc1UVf-VE",
            "javascript:alert(1)",
            "https://youtube.com/playlist?list=123",
        ] {
            assert!(video_id(input).is_none(), "{input}");
        }
    }

    #[test]
    fn data_redirects_stay_within_the_youtube_origin() {
        let base = Url::parse(ORIGIN).unwrap();
        assert!(safe_data_origin(
            &base.join("/@BlenderFoundation/videos").unwrap()
        ));
        assert!(!safe_data_origin(
            &base
                .join("https://www.youtube.com.evil.test/watch")
                .unwrap()
        ));
        assert!(!safe_data_origin(
            &base.join("https://consent.youtube.com/").unwrap()
        ));
    }

    #[test]
    fn extracts_unique_videos_and_shorts_without_ad_cards() {
        let response = json!({"contents": {"results": [
            {"adSlotRenderer": {"videoRenderer":{"videoId":"sponsored00","title":{"simpleText":"Ad"}}}},
            {"videoRenderer": {
                "videoId":"M7lc1UVf-VE",
                "title":{"runs":[{"text":"Rust "},{"text":"on Android"}]},
                "ownerText":{"simpleText":"Channel"},
                "lengthText":{"simpleText":"12:34"}
            }},
            {"videoRenderer":{"videoId":"M7lc1UVf-VE","title":{"simpleText":"Duplicate"}}},
            {"shortsLockupViewModel": {
                "onTap":{"innertubeCommand":{"reelWatchEndpoint":{"videoId":"H6ML7P19ySw"}}},
                "overlayMetadata":{"primaryText":{"content":"A Short"},"secondaryText":{"content":"123 views"}}
            }}
        ]}});
        let videos = parse_search_results(&response, false, None).unwrap();
        assert_eq!(videos.len(), 1);
        assert_eq!(videos[0].title, "Rust on Android");
        assert_eq!(videos[0].channel.name, "Channel");
        assert_eq!(videos[0].duration, "12:34");
        let shorts = parse_search_results(&response, true, None).unwrap();
        assert_eq!(shorts.len(), 1);
        assert_eq!(shorts[0].title, "A Short");
    }
}
