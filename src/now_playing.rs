//! macOS Now Playing: the video in Control Center's and the menu bar's Now
//! Playing widget and on the lock screen, with its thumbnail and position, and
//! play / pause / next / previous / seek from there, the keyboard's media keys
//! and headphones (macOS routes those to the Now Playing app).
//! The Linux counterpart is `mpris`, the Windows one `media_controls`.
use crate::media_session::{self, Clock, Command, Snapshot, State};
use block2::RcBlock;
use futures::{StreamExt, channel::mpsc};
use gpui_kit::App;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSImage;
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSMutableDictionary, NSNumber, NSString};
use objc2_media_player::{
    MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyArtist,
    MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration, MPMediaItemPropertyTitle,
    MPNowPlayingInfoCenter, MPNowPlayingInfoMediaType, MPNowPlayingInfoPropertyElapsedPlaybackTime,
    MPNowPlayingInfoPropertyIsLiveStream, MPNowPlayingInfoPropertyMediaType,
    MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommand,
    MPRemoteCommandCenter, MPRemoteCommandEvent, MPRemoteCommandHandlerStatus,
};
use std::cell::RefCell;
use std::ptr::NonNull;
use std::rc::Rc;

/// What was last handed to the system, all on the main thread.
#[derive(Default)]
struct Published {
    snapshot: Option<Snapshot>,
    clock: Clock,
    /// The video id and its thumbnail.
    art: Option<(String, Retained<MPMediaItemArtwork>)>,
}

/// The remote commands whose availability follows the video.
struct Commands {
    next: Retained<MPRemoteCommand>,
    previous: Retained<MPRemoteCommand>,
    seek: Retained<MPRemoteCommand>,
}

fn publish(state: &Published, time: f64, commands: &Commands) {
    // SAFETY: called on the main thread; the keys are MediaPlayer's own constants
    unsafe {
        let center = MPNowPlayingInfoCenter::defaultCenter();
        let Some(s) = &state.snapshot else {
            center.setNowPlayingInfo(None);
            center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
            return;
        };
        let info = NSMutableDictionary::<NSString, AnyObject>::new();
        info.insert(MPMediaItemPropertyTitle, &NSString::from_str(&s.title));
        info.insert(MPMediaItemPropertyArtist, &NSString::from_str(&s.channel));
        info.insert(
            MPNowPlayingInfoPropertyMediaType,
            &NSNumber::numberWithUnsignedInteger(MPNowPlayingInfoMediaType::Video.0),
        );
        info.insert(
            MPNowPlayingInfoPropertyIsLiveStream,
            &NSNumber::numberWithBool(s.live),
        );
        if s.duration > 0.0 {
            info.insert(
                MPMediaItemPropertyPlaybackDuration,
                &NSNumber::numberWithDouble(s.duration),
            );
        }
        info.insert(
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            &NSNumber::numberWithDouble(time.max(0.0)),
        );
        info.insert(
            MPNowPlayingInfoPropertyPlaybackRate,
            &NSNumber::numberWithDouble(s.rate),
        );
        if let Some((_, art)) = state.art.as_ref().filter(|(id, _)| *id == s.id) {
            info.insert(MPMediaItemPropertyArtwork, art);
        }
        center.setNowPlayingInfo(Some(&info));
        center.setPlaybackState(match s.state {
            State::Playing => MPNowPlayingPlaybackState::Playing,
            State::Paused => MPNowPlayingPlaybackState::Paused,
            State::Stopped => MPNowPlayingPlaybackState::Stopped,
        });
        commands.next.setEnabled(s.can_next);
        commands.previous.setEnabled(s.can_previous);
        commands.seek.setEnabled(s.can_seek());
    }
}

/// The thumbnail as Now Playing artwork, built on the main thread.
fn artwork(bytes: &[u8]) -> Option<Retained<MPMediaItemArtwork>> {
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
    let size: CGSize = image.size();
    let handler = RcBlock::new(move |_: CGSize| NonNull::from(&*image));
    // SAFETY: the block keeps the image alive, so the returned pointer stays valid
    Some(unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            size,
            &handler,
        )
    })
}

/// Sends `command` (or what `of_event` makes of the event) to the app when the
/// system's remote command fires.
fn on_command(
    command: &MPRemoteCommand,
    send: &mpsc::UnboundedSender<Command>,
    of_event: impl Fn(&MPRemoteCommandEvent) -> Option<Command> + 'static,
) {
    let send = send.clone();
    let handler = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
        // SAFETY: MediaPlayer passes a live event for the duration of the call
        match of_event(unsafe { event.as_ref() }) {
            Some(command) if send.unbounded_send(command).is_ok() => {
                MPRemoteCommandHandlerStatus::Success
            }
            _ => MPRemoteCommandHandlerStatus::CommandFailed,
        }
    });
    // SAFETY: main thread; the command center keeps the handler for the app's lifetime
    unsafe {
        command.addTargetWithHandler(&handler);
        command.setEnabled(true);
    }
}

fn register(send: &mpsc::UnboundedSender<Command>) -> Commands {
    // SAFETY: main thread
    let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
    let fixed = |c: Command| move |_: &MPRemoteCommandEvent| Some(c);
    unsafe {
        on_command(&center.playCommand(), send, fixed(Command::Play));
        on_command(&center.pauseCommand(), send, fixed(Command::Pause));
        on_command(
            &center.togglePlayPauseCommand(),
            send,
            fixed(Command::Toggle),
        );
        on_command(&center.stopCommand(), send, fixed(Command::Stop));
        let next = center.nextTrackCommand();
        on_command(&next, send, fixed(Command::Next));
        let previous = center.previousTrackCommand();
        on_command(&previous, send, fixed(Command::Previous));
        let seek: Retained<MPRemoteCommand> =
            Retained::into_super(center.changePlaybackPositionCommand());
        on_command(&seek, send, |event| {
            let event = event.downcast_ref::<MPChangePlaybackPositionCommandEvent>()?;
            Some(Command::Seek(event.positionTime()))
        });
        Commands {
            next,
            previous,
            seek,
        }
    }
}

pub(crate) fn init(cx: &mut App) {
    let (send, mut received) = mpsc::unbounded();
    let commands = Rc::new(register(&send));
    let state = Rc::new(RefCell::new(Published::default()));
    // thumbnails are downloaded off the main thread and turned into artwork on it
    let (art_send, mut art_received) = mpsc::unbounded::<(String, Vec<u8>)>();

    let (follow_state, follow_commands) = (state.clone(), commands.clone());
    media_session::follow(cx, move |snap, time, _| {
        let mut state = follow_state.borrow_mut();
        let rate = snap.as_ref().map_or(0.0, |s| s.rate);
        let changed = snap != state.snapshot;
        if !changed && !state.clock.drifted(rate, time) {
            return;
        }
        let new_video = snap
            .as_ref()
            .map(|s| s.id.clone())
            .filter(|id| state.snapshot.as_ref().map(|s| &s.id) != Some(id));
        state.clock.set(time);
        state.snapshot = snap;
        publish(&state, time, &follow_commands);
        if let Some(id) = new_video {
            let art_send = art_send.clone();
            std::thread::spawn(move || {
                if let Ok(bytes) =
                    crate::api::http::get_bytes(&media_session::thumbnail_url(&id), &[])
                {
                    let _ = art_send.unbounded_send((id, bytes));
                }
            });
        }
    });

    let art_state = state.clone();
    cx.spawn(async move |_| {
        while let Some((id, bytes)) = art_received.next().await {
            let mut state = art_state.borrow_mut();
            if state.snapshot.as_ref().is_none_or(|s| s.id != id) {
                continue;
            }
            let Some(art) = artwork(&bytes) else { continue };
            state.art = Some((id, art));
            let time = state
                .snapshot
                .as_ref()
                .map_or(0.0, |s| state.clock.position(s.rate));
            publish(&state, time, &commands);
        }
    })
    .detach();

    cx.spawn(async move |cx| {
        while let Some(command) = received.next().await {
            let _ = cx.update(|cx| media_session::apply(command, cx));
        }
    })
    .detach();

    cx.on_app_quit(|_| {
        // SAFETY: main thread
        unsafe { MPNowPlayingInfoCenter::defaultCenter().setNowPlayingInfo(None) };
        std::future::ready(())
    })
    .detach();
}
