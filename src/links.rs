//! A single running instance: a second launch (e.g. with a YouTube link on the
//! command line) hands its argument to the first and exits.
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

/// Turns a `streamtube:` link back into the YouTube URL it wraps.
/// Accepts `streamtube://open?url=<encoded>`, `streamtube:https://…` and
/// `streamtube://www.youtube.com/watch?v=…`; anything else is returned unchanged.
pub fn normalize(arg: &str) -> String {
    let a = arg.trim().trim_matches('"');
    let Some(rest) = a.strip_prefix("streamtube:").or_else(|| a.strip_prefix("STREAMTUBE:")) else {
        return a.to_string();
    };
    let rest = rest.trim_start_matches('/');
    if let Some(q) = rest.strip_prefix("open?").or_else(|| rest.strip_prefix("open/?")) {
        if let Some(u) = q.split('&').find_map(|kv| kv.strip_prefix("url=")) {
            return crate::api::http::urldecode(u);
        }
        if let Some(v) = q.split('&').find_map(|kv| kv.strip_prefix("v=")) {
            return format!("https://www.youtube.com/watch?v={v}");
        }
    }
    if rest.starts_with("http:") || rest.starts_with("https:") {
        // "https:/www…" when a browser collapsed the slashes
        let (scheme, tail) = rest.split_once(':').unwrap_or(("https", ""));
        return format!("{scheme}://{}", tail.trim_start_matches('/'));
    }
    format!("https://{rest}")
}

fn port_file() -> std::path::PathBuf {
    crate::store::data_dir().join("instance")
}

/// Whether another copy of the app is running, going by the file it wrote at start.
pub fn other_instance_running() -> bool {
    let Ok(s) = std::fs::read_to_string(port_file()) else { return false };
    let pid = s.split_whitespace().nth(1).and_then(|p| p.parse::<u32>().ok());
    pid.is_some_and(|p| p != std::process::id() && process_alive(p))
}

/// Hands `arg` to an already running instance. True when one took it (this process should exit).
pub fn forward_to_running(arg: &str) -> bool {
    let Ok(s) = std::fs::read_to_string(port_file()) else { return false };
    let mut parts = s.split_whitespace();
    let (Some(port), pid) = (parts.next().and_then(|p| p.parse::<u16>().ok()), parts.next().and_then(|p| p.parse::<u32>().ok())) else {
        return false;
    };
    // the running instance only takes links that carry the secret from this file
    let token = parts.next().unwrap_or_default();
    // Left over from an instance that has exited: connecting to its closed port
    // only fails after the timeout on Windows, which delayed every launch.
    if pid.is_some_and(|p| !process_alive(p)) {
        return false;
    }
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else { return false };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
    #[cfg(windows)]
    if let Some(pid) = pid {
        // let the running instance bring its window to the front
        #[link(name = "user32")]
        unsafe extern "system" {
            fn AllowSetForegroundWindow(pid: u32) -> i32;
        }
        unsafe { AllowSetForegroundWindow(pid) };
    }
    #[cfg(not(windows))]
    let _ = pid;
    if stream.write_all(format!("STREAMTUBE {token} {}\n", arg.replace('\n', " ")).as_bytes()).is_err() {
        return false;
    }
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).is_ok() && reply.trim() == "ok"
}

/// What to do with the address another program asked us to open.
#[derive(Clone, Debug, PartialEq)]
pub enum Launch {
    /// Open the app. `url` is a YouTube page when the address was one.
    App { url: Option<String> },
    /// This is not a YouTube page. Hand it to the real browser and do not open a window.
    Browser(String),
}

/// YouTube http(s) links open in the app. Every other http(s) link, and an `.html`
/// file Windows may pass because we registered as a browser, goes to the real browser.
pub fn classify_launch(arg: &str) -> Launch {
    let arg = normalize(arg);
    if arg.is_empty() {
        return Launch::App { url: None };
    }
    if let Some(url) = crate::api::innertube::http_target(&arg) {
        if crate::api::innertube::is_youtube_url(&url) {
            return Launch::App { url: Some(url) };
        }
        return Launch::Browser(url);
    }
    let lower = arg.to_ascii_lowercase();
    if lower.ends_with(".htm") || lower.ends_with(".html") || lower.ends_with(".xhtml") {
        return Launch::Browser(arg);
    }
    Launch::App { url: None }
}

/// The YouTube page a launch should open, after redirect links are unwrapped.
pub fn app_url(arg: &str) -> Option<String> {
    match classify_launch(arg) {
        Launch::App { url } => url,
        Launch::Browser(_) => None,
    }
}

fn browser_file() -> std::path::PathBuf {
    crate::store::data_dir().join("https-browser.txt")
}

/// The command line Windows is using for https, when it is not this app.
#[cfg(windows)]
fn current_browser_command() -> Option<String> {
    let progid = reg_value(&["query", r"HKCU\Software\Microsoft\Windows\Shell\Associations\UrlAssociations\https\UserChoice", "/v", "ProgId"])?;
    if progid.eq_ignore_ascii_case("StreamTubeHTML") {
        return None;
    }
    prog_command(&progid).filter(|cmd| !command_targets_exe(cmd, "streamtube.exe"))
}

/// Remember the https handler so non-YouTube links can be handed back to it.
/// Once Stream Tube itself is that handler, the saved command is left as it was.
#[cfg(windows)]
pub fn remember_browser() {
    let Some(cmd) = current_browser_command() else { return };
    if cmd.contains('\n') || cmd.contains('\r') {
        return;
    }
    let _ = std::fs::write(browser_file(), cmd);
}

#[cfg(not(windows))]
pub fn remember_browser() {}

/// Opens `url` in the browser that was default before Stream Tube, by running its
/// program directly. Going through https again would start this app once more.
pub fn open_in_browser(url: &str) -> bool {
    if url.contains(['\n', '\r', '"']) {
        return false;
    }
    #[cfg(windows)]
    {
        let saved = std::fs::read_to_string(browser_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let fallback = fallback_browser();
        for template in [saved, fallback].into_iter().flatten() {
            if command_targets_exe(&template, "streamtube.exe") {
                continue;
            }
            let Some((exe, args)) = browser_launch(&template, url) else { continue };
            if command_targets_exe(&exe, "streamtube.exe") {
                continue;
            }
            if std::process::Command::new(exe).args(args).spawn().is_ok() {
                return true;
            }
        }
        return false;
    }
    #[cfg(not(windows))]
    {
        open::that(url).is_ok()
    }
}

/// `template` is a shell open command (`"chrome.exe" --single-argument %1`).
fn browser_launch(template: &str, url: &str) -> Option<(String, Vec<String>)> {
    let mut parts = split_command(template);
    if parts.is_empty() {
        return None;
    }
    let mut replaced = false;
    for part in parts.iter_mut().skip(1) {
        if part == "%1" {
            *part = url.to_string();
            replaced = true;
        }
    }
    if !replaced {
        parts.push(url.to_string());
    }
    let exe = parts.remove(0);
    Some((exe, parts))
}

fn split_command(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = false;
    for c in command.chars() {
        match c {
            '"' => quote = !quote,
            ' ' | '\t' if !quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn command_targets_exe(command: &str, exe_name: &str) -> bool {
    split_command(command)
        .first()
        .and_then(|path| std::path::Path::new(path).file_name())
        .is_some_and(|name| name.eq_ignore_ascii_case(exe_name))
}

#[cfg(windows)]
fn fallback_browser() -> Option<String> {
    for progid in ["ChromeHTML", "MSEdgeHTM", "BraveHTML", "FirefoxURL"] {
        if let Some(cmd) = prog_command(progid).filter(|cmd| !command_targets_exe(cmd, "streamtube.exe")) {
            return Some(cmd);
        }
    }
    let edge = std::path::PathBuf::from(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe");
    edge.exists().then(|| format!("\"{}\" --single-argument %1", edge.display()))
}

#[cfg(windows)]
fn prog_command(progid: &str) -> Option<String> {
    for root in [r"HKCU\Software\Classes", r"HKLM\Software\Classes"] {
        let key = format!(r"{root}\{progid}\shell\open\command");
        if let Some(cmd) = reg_value(&["query", &key, "/ve"]) {
            return Some(cmd);
        }
    }
    None
}

#[cfg(windows)]
fn reg_value(args: &[&str]) -> Option<String> {
    let out = reg(args)?;
    for line in out.lines() {
        let Some((_, rest)) = line.split_once("REG_SZ") else { continue };
        let value = rest.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(windows)]
fn reg(args: &[&str]) -> Option<String> {
    let mut c = std::process::Command::new("reg");
    c.args(args);
    crate::api::streams::hide_window(&mut c);
    let out = c.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Registers Stream Tube as a web-link handler and, when `open_settings` is set,
/// opens the Windows page where the user confirms that choice. Windows will not
/// switch the default by itself.
#[cfg(windows)]
pub fn install_https_handler(open_settings: bool) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = exe.to_string_lossy().to_string();
    let open_cmd = format!("\"{exe}\" \"%1\"");
    let icon = format!("\"{exe}\",0");
    let class = r"HKCU\Software\Classes\StreamTubeHTML";
    let client = r"HKCU\Software\Clients\StartMenuInternet\StreamTube";
    let ok = reg(&["add", class, "/ve", "/d", "Stream Tube", "/f"]).is_some()
        && reg(&["add", class, "/v", "URL Protocol", "/d", "", "/f"]).is_some()
        && reg(&["add", &format!(r"{class}\DefaultIcon"), "/ve", "/d", &icon, "/f"]).is_some()
        && reg(&["add", &format!(r"{class}\shell\open\command"), "/ve", "/d", &open_cmd, "/f"]).is_some()
        && reg(&["add", client, "/ve", "/d", "Stream Tube", "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\shell\open\command"), "/ve", "/d", &format!("\"{exe}\""), "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\Capabilities"), "/v", "ApplicationName", "/d", "Stream Tube", "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\Capabilities"), "/v", "ApplicationDescription", "/d", "Opens YouTube links. Other links open in your browser.", "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\Capabilities"), "/v", "ApplicationIcon", "/d", &icon, "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\Capabilities\URLAssociations"), "/v", "http", "/d", "StreamTubeHTML", "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\Capabilities\URLAssociations"), "/v", "https", "/d", "StreamTubeHTML", "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\Capabilities\FileAssociations"), "/v", ".htm", "/d", "StreamTubeHTML", "/f"]).is_some()
        && reg(&["add", &format!(r"{client}\Capabilities\FileAssociations"), "/v", ".html", "/d", "StreamTubeHTML", "/f"]).is_some()
        && reg(&["add", r"HKCU\Software\RegisteredApplications", "/v", "StreamTube", "/d", r"Software\Clients\StartMenuInternet\StreamTube\Capabilities", "/f"]).is_some();
    if ok && open_settings {
        let _ = open::that("ms-settings:defaultapps?registeredAppUser=StreamTube");
    }
    ok
}

#[cfg(not(windows))]
pub fn install_https_handler(_open_settings: bool) -> bool {
    false
}

#[cfg(windows)]
pub fn remove_https_handler() {
    let _ = reg(&["delete", r"HKCU\Software\Classes\StreamTubeHTML", "/f"]);
    let _ = reg(&["delete", r"HKCU\Software\Clients\StartMenuInternet\StreamTube", "/f"]);
    let _ = reg(&["delete", r"HKCU\Software\RegisteredApplications", "/v", "StreamTube", "/f"]);
}

#[cfg(not(windows))]
pub fn remove_https_handler() {}

/// Makes `streamtube://` links open this app when nothing handles them yet (the
/// portable zip; the installer registers them itself). Runs in the background.
pub fn register_scheme() {
    #[cfg(windows)]
    std::thread::spawn(|| {
        use std::process::Command;
        let Ok(exe) = std::env::current_exe() else { return };
        let key = r"HKCU\Software\Classes\streamtube";
        let reg = |args: &[&str]| {
            let mut c = Command::new("reg");
            c.args(args);
            crate::api::streams::hide_window(&mut c);
            c.output().map(|o| o.status.success()).unwrap_or(false)
        };
        if reg(&["query", &format!(r"{key}\shell\open\command"), "/ve"]) {
            return; // registered already (by the installer or an earlier run)
        }
        let exe = exe.to_string_lossy();
        let cmd = format!("\"{exe}\" \"%1\"");
        let _ = reg(&["add", key, "/ve", "/d", "URL:Stream Tube", "/f"])
            && reg(&["add", key, "/v", "URL Protocol", "/d", "", "/f"])
            && reg(&["add", &format!(r"{key}\DefaultIcon"), "/ve", "/d", &format!("{exe},0"), "/f"])
            && reg(&["add", &format!(r"{key}\shell\open\command"), "/ve", "/d", &cmd, "/f"]);
    });
}

/// Whether a process with this id is running.
pub(crate) fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const STILL_ACTIVE: u32 = 259;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
            fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
            fn CloseHandle(handle: isize) -> i32;
        }
        // SAFETY: plain Win32 calls on a handle we open and close here
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h == 0 {
                return false;
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code) != 0;
            CloseHandle(h);
            !ok || code == STILL_ACTIVE
        }
    }
    #[cfg(not(windows))]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists() || !std::path::Path::new("/proc/self").exists()
    }
}

/// Listens for links from later launches; each arrives on the returned channel.
pub fn listen() -> Option<futures::channel::mpsc::UnboundedReceiver<String>> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    // Any local process can reach a loopback port, so a link is only taken with this
    // secret. It lives in the profile folder, which only this user can read.
    let token = new_token();
    std::fs::write(port_file(), format!("{port} {} {token}", std::process::id())).ok()?;
    let (tx, rx) = futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut line = String::new();
            // a link is short; don't let another local process feed us endless bytes
            let mut reader = BufReader::new(std::io::Read::take(&stream, 16 * 1024));
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            // only our own launches speak this, and know the secret
            let Some(arg) = parse_request(&line, &token) else { continue };
            let _ = (&stream).write_all(b"ok\n");
            if tx.unbounded_send(arg.to_string()).is_err() {
                break;
            }
        }
    });
    Some(rx)
}

fn parse_request<'a>(line: &'a str, token: &str) -> Option<&'a str> {
    let payload = line.trim_end_matches(['\r', '\n']).strip_prefix("STREAMTUBE ")?;
    check_token(payload, token)
}

/// A random secret for the loopback link port.
fn new_token() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..32).map(|_| format!("{:02x}", rng.random::<u8>())).collect()
}

/// The link after "<token> " when the token matches (compared in constant time).
fn check_token<'a>(rest: &'a str, token: &str) -> Option<&'a str> {
    let (given, arg) = rest.split_once(' ')?;
    let same = given.len() == token.len() && given.bytes().zip(token.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
    same.then_some(arg)
}

#[cfg(test)]
mod tests {
    use super::{browser_launch, classify_launch, command_targets_exe, normalize, Launch};

    #[test]
    fn links_need_the_token() {
        let token = super::new_token();
        assert_eq!(token.len(), 64);
        assert_eq!(super::check_token(&format!("{token} https://youtu.be/abc"), &token), Some("https://youtu.be/abc"));
        assert_eq!(super::check_token("https://youtu.be/abc", &token), None);
        assert_eq!(super::check_token(&format!("{} https://youtu.be/abc", "0".repeat(64)), &token), None);
        assert_ne!(super::new_token(), token);
    }

    #[test]
    fn launch_without_a_url_is_forwarded_to_the_running_instance() {
        let token = super::new_token();
        assert_eq!(super::parse_request(&format!("STREAMTUBE {token} \n"), &token), Some(""));
        assert_eq!(super::parse_request(&format!("STREAMTUBE {token}\n"), &token), None);
    }

    #[test]
    fn alive() {
        assert!(super::process_alive(std::process::id()));
        assert!(!super::process_alive(u32::MAX - 1));
    }

    #[test]
    fn youtube_https_opens_in_the_app_and_other_links_do_not() {
        assert_eq!(
            classify_launch("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Launch::App { url: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()) }
        );
        assert_eq!(
            classify_launch("https://youtu.be/dQw4w9WgXcQ"),
            Launch::App { url: Some("https://youtu.be/dQw4w9WgXcQ".into()) }
        );
        assert_eq!(
            classify_launch("https://www.youtube.com/redirect?q=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3DdQw4w9WgXcQ"),
            Launch::App { url: Some("https://www.youtube.com/watch?v=dQw4w9WgXcQ".into()) }
        );
        assert_eq!(
            classify_launch("https://www.youtube.com/redirect?q=https%3A%2F%2Fexample.com%2Fa&v=abcdefghijk"),
            Launch::Browser("https://example.com/a".into())
        );
        assert_eq!(classify_launch("https://example.com/a"), Launch::Browser("https://example.com/a".into()));
        assert_eq!(classify_launch(r"C:\page.html"), Launch::Browser(r"C:\page.html".into()));
        assert_eq!(classify_launch(""), Launch::App { url: None });
        assert!(command_targets_exe(r#""C:\Apps\streamtube.exe" "%1""#, "streamtube.exe"));
        assert!(!command_targets_exe(r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1"#, "streamtube.exe"));
        assert_eq!(
            browser_launch(r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1"#, "https://example.com/a"),
            Some(("C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe".into(), vec!["--single-argument".into(), "https://example.com/a".into()]))
        );
    }

    #[test]
    fn scheme_links() {
        assert_eq!(normalize("streamtube://open?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3Dabc"), "https://www.youtube.com/watch?v=abc");
        assert_eq!(normalize("streamtube:https://youtu.be/abc"), "https://youtu.be/abc");
        assert_eq!(normalize("streamtube://https:/youtu.be/abc"), "https://youtu.be/abc");
        assert_eq!(normalize("streamtube://www.youtube.com/watch?v=abc"), "https://www.youtube.com/watch?v=abc");
        assert_eq!(normalize("https://youtu.be/abc"), "https://youtu.be/abc");
    }
}
