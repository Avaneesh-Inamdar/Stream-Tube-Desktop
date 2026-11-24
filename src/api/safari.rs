//! A single, user-initiated watch-history connection from our Safari extension.
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use objc2::{msg_send, runtime::AnyClass};
use objc2_foundation::{NSDictionary, NSString};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const EXTENSION_ID: &str = "io.github.avaneesh-inamdar.StreamTube.SafariPrototype.Extension";
const EXTENSION_BUNDLE: &str = "Stream Tube Safari History Extension.appex";
const MAX_PAYLOAD: u64 = 16_384;
const TIMEOUT: Duration = Duration::from_secs(300);

#[link(name = "SafariServices", kind = "framework")]
unsafe extern "C" {}

pub fn available() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.parent()?
                .parent()
                .map(|contents| contents.join("PlugIns").join(EXTENSION_BUNDLE))
        })
        .is_some_and(|extension| extension.is_dir())
}

#[derive(Clone)]
pub struct Request {
    token: String,
    port: u16,
}

impl Request {
    /// Called on the main thread while the in-app connection dialog is open.
    pub fn notify_extension(&self) {
        let Some(application) = AnyClass::get(c"SFSafariApplication") else {
            return;
        };
        let name = NSString::from_str("watch-history-request");
        let identifier = NSString::from_str(EXTENSION_ID);
        let token_key = NSString::from_str("token");
        let port_key = NSString::from_str("port");
        let token = NSString::from_str(&self.token);
        let port = NSString::from_str(&self.port.to_string());
        let payload = NSDictionary::from_slices(&[&*token_key, &*port_key], &[&*token, &*port]);
        // SAFETY: These objects and the optional nil completion match the SafariServices API.
        unsafe {
            let _: () = msg_send![
                application,
                dispatchMessageWithName: &*name,
                toExtensionWithIdentifier: &*identifier,
                userInfo: &*payload,
                completionHandler: std::ptr::null::<std::ffi::c_void>()
            ];
        }
    }
}

pub struct Pending {
    listener: TcpListener,
    request: Request,
}

impl Pending {
    pub fn new() -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
        let port = listener.local_addr()?.port();
        Ok(Self {
            listener,
            request: Request { token, port },
        })
    }

    pub fn request(&self) -> Request {
        self.request.clone()
    }

    pub fn open_safari() -> Result<()> {
        let status = std::process::Command::new("/usr/bin/open")
            .args(["-b", "com.apple.Safari", "https://www.youtube.com/"])
            .status()
            .context("Could not open Safari")?;
        if !status.success() {
            bail!("Could not open Safari");
        }
        Ok(())
    }

    pub fn wait(self, cancel: &AtomicBool) -> Result<String> {
        let deadline = Instant::now() + TIMEOUT;
        while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let Some(cookies) = read_session(&stream, &self.request.token) else {
                        continue;
                    };
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let _ = stream.write_all(&[1]);
                    drop(self.listener);
                    return super::web::connect_safari(&cookies, cancel);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(80));
                }
                Err(error) => return Err(error.into()),
            }
        }
        bail!(
            "Safari connection ended. Open the extension on a signed-in YouTube tab and try again."
        )
    }
}

fn read_session(stream: &TcpStream, token: &str) -> Option<Vec<Value>> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .ok()?;
    let mut reader = BufReader::new(stream);
    let mut challenge = [0_u8; 44];
    reader.read_exact(&mut challenge).ok()?;
    let (terminator, presented) = challenge.split_last()?;
    let mismatch = presented
        .iter()
        .zip(token.bytes())
        .fold(0, |acc, (a, b)| acc | (a ^ b));
    if mismatch != 0 || *terminator != b'\n' {
        return None;
    }
    let mut payload = Vec::new();
    reader
        .take(MAX_PAYLOAD + 1)
        .read_until(b'\n', &mut payload)
        .ok()?;
    if payload.last() != Some(&b'\n') {
        return None;
    }
    serde_json::from_slice(&payload).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn handoff_requires_the_current_token() {
        let pending = Pending::new().unwrap();
        pending.listener.set_nonblocking(false).unwrap();
        let correct = pending.request();
        let other = Pending::new().unwrap().request();
        let mut send = TcpStream::connect(("127.0.0.1", correct.port)).unwrap();
        let (received, _) = pending.listener.accept().unwrap();
        let payload = json!([{"name": "TEST", "domain": ".youtube.com", "value": "synthetic"}]);
        writeln!(send, "{}", other.token).unwrap();
        writeln!(send, "{payload}").unwrap();
        assert!(read_session(&received, &correct.token).is_none());

        let mut send = TcpStream::connect(("127.0.0.1", correct.port)).unwrap();
        let (received, _) = pending.listener.accept().unwrap();
        writeln!(send, "{}", correct.token).unwrap();
        writeln!(send, "{payload}").unwrap();
        assert_eq!(
            json!(read_session(&received, &correct.token).unwrap()),
            payload
        );
    }

    #[test]
    fn handoff_rejects_an_incomplete_or_oversized_payload() {
        for size in [8, MAX_PAYLOAD as usize + 1] {
            let pending = Pending::new().unwrap();
            pending.listener.set_nonblocking(false).unwrap();
            let mut send = TcpStream::connect(("127.0.0.1", pending.request.port)).unwrap();
            let (received, _) = pending.listener.accept().unwrap();
            writeln!(send, "{}", pending.request.token).unwrap();
            send.write_all(&vec![b'a'; size]).unwrap();
            drop(send);
            assert!(read_session(&received, &pending.request.token).is_none());
        }
    }
}
