//! Publishes the player over MPRIS (`org.mpris.MediaPlayer2.streamtube`) so GNOME, KDE Plasma and
//! phone remotes such as KDE Connect show a media card with transport controls. This is the Linux
//! counterpart of the Windows SMTC integration in media_controls.rs.
use crate::player::controller::{Player, PlayerHandle, player};
use futures::{StreamExt, channel::mpsc};
use gpui_kit::{App, Entity, Subscription};
use mpris_server::{Metadata, PlaybackStatus, Player as Mpris, Time, TrackId};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Closed,
    Stopped,
    Paused,
    Playing,
}

#[derive(Clone, PartialEq)]
struct Video {
    id: String,
    title: String,
    channel: String,
    thumbnail: String,
    /// whole seconds: mpv refines the duration while loading
    duration: u64,
}

#[derive(Clone)]
struct State {
    video: Option<Video>,
    status: Status,
    next: bool,
    previous: bool,
    speed: f64,
    time: f64,
}

impl State {
    fn of(player: &Player) -> State {
        let video = player.video.as_ref().map(|v| Video {
            id: v.id.clone(),
            title: v.title.clone(),
            channel: v.channel_name.clone(),
            thumbnail: v.thumbnail.clone(),
            duration: player.duration.max(0.0) as u64,
        });
        let status = if video.is_none() || player.error.is_some() {
            Status::Closed
        } else if player.eof {
            Status::Stopped
        } else if player.paused {
            Status::Paused
        } else {
            Status::Playing
        };
        State {
            video,
            status,
            next: player.queue_pos.is_some_and(|i| i + 1 < player.queue.len()),
            previous: player.queue_pos.is_some_and(|i| i > 0),
            speed: player.speed,
            time: player.time.max(0.0),
        }
    }

    /// The player notifies on every frame; only send what a remote can't extrapolate.
    fn worth_sending(&self, last: &State) -> bool {
        self.video != last.video
            || self.status != last.status
            || self.next != last.next
            || self.previous != last.previous
            || self.speed != last.speed
            || (self.time - last.time).abs() >= 1.0
    }
}

enum Command {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
    SeekBy(f64),
    SeekTo(f64),
}

fn micros(seconds: f64) -> Time {
    Time::from_micros((seconds * 1_000_000.0) as i64)
}

fn seconds(time: Time) -> f64 {
    time.as_micros() as f64 / 1_000_000.0
}

fn track_id(video: &Option<Video>) -> TrackId {
    let Some(video) = video else { return TrackId::NO_TRACK };
    let id: String = video.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
    TrackId::try_from(format!("/io/github/avaneesh_inamdar/StreamTube/video/{id}")).unwrap_or(TrackId::NO_TRACK)
}

/// The D-Bus side: an MPRIS server on its own thread, fed with player snapshots and sending
/// remote button presses back.
struct Remote {
    mpris: Mpris,
    track: Rc<RefCell<TrackId>>,
    last: Option<(State, Instant)>,
}

impl Remote {
    async fn publish(&mut self, state: State) {
        let last = self.last.as_ref().map(|(s, _)| s);
        if last.map(|s| &s.video) != Some(&state.video) {
            let track = track_id(&state.video);
            *self.track.borrow_mut() = track.clone();
            let metadata = match &state.video {
                Some(v) => {
                    let mut m = Metadata::builder().trackid(track).title(v.title.clone()).artist([v.channel.clone()]);
                    if v.duration > 0 {
                        m = m.length(Time::from_secs(v.duration as i64));
                    }
                    if v.thumbnail.starts_with("https://") {
                        m = m.art_url(v.thumbnail.clone());
                    }
                    m.build()
                }
                None => Metadata::new(),
            };
            let _ = self.mpris.set_metadata(metadata).await;
        }
        if last.map(|s| s.status) != Some(state.status) {
            let open = state.status != Status::Closed;
            let _ = self.mpris.set_playback_status(match state.status {
                Status::Closed | Status::Stopped => PlaybackStatus::Stopped,
                Status::Paused => PlaybackStatus::Paused,
                Status::Playing => PlaybackStatus::Playing,
            }).await;
            let _ = self.mpris.set_can_play(open).await;
            let _ = self.mpris.set_can_pause(open).await;
            let _ = self.mpris.set_can_seek(open).await;
        }
        if last.map(|s| s.next) != Some(state.next) {
            let _ = self.mpris.set_can_go_next(state.next).await;
        }
        if last.map(|s| s.previous) != Some(state.previous) {
            let _ = self.mpris.set_can_go_previous(state.previous).await;
        }
        if last.map(|s| s.speed) != Some(state.speed) {
            let _ = self.mpris.set_maximum_rate(state.speed.max(1.0)).await;
            let _ = self.mpris.set_minimum_rate(state.speed.min(1.0)).await;
            let _ = self.mpris.set_rate(state.speed).await;
        }
        // Remotes extrapolate the position from Rate, so a jump means the user seeked.
        let jumped = match &self.last {
            Some((s, at)) if s.video == state.video => {
                let expected = if s.status == Status::Playing { s.time + at.elapsed().as_secs_f64() * s.speed } else { s.time };
                (state.time - expected).abs() > 1.5
            }
            _ => false,
        };
        self.mpris.set_position(micros(state.time));
        if jumped {
            let _ = self.mpris.seeked(micros(state.time)).await;
        }
        self.last = Some((state, Instant::now()));
    }
}

fn spawn_remote(mut states: mpsc::UnboundedReceiver<State>, commands: mpsc::UnboundedSender<Command>) {
    let thread = std::thread::Builder::new().name("mpris".into()).spawn(move || {
        futures::executor::block_on(async move {
            // Fails when another instance already owns the name; that one keeps the card.
            let Ok(mpris) = Mpris::builder("streamtube")
                .identity("Stream Tube")
                .desktop_entry("streamtube")
                .can_control(true)
                .build()
                .await
            else {
                return;
            };
            let track = Rc::new(RefCell::new(TrackId::NO_TRACK));
            let send = |command: fn() -> Command| {
                let commands = commands.clone();
                move |_: &Mpris| {
                    let _ = commands.unbounded_send(command());
                }
            };
            mpris.connect_play(send(|| Command::Play));
            mpris.connect_pause(send(|| Command::Pause));
            mpris.connect_play_pause(send(|| Command::PlayPause));
            mpris.connect_next(send(|| Command::Next));
            mpris.connect_previous(send(|| Command::Previous));
            let sender = commands.clone();
            mpris.connect_seek(move |_, offset| {
                let _ = sender.unbounded_send(Command::SeekBy(seconds(offset)));
            });
            let sender = commands.clone();
            let current = track.clone();
            mpris.connect_set_position(move |_, id, position| {
                // the spec says to ignore requests for a track that is no longer playing
                if *id == *current.borrow() && position >= Time::ZERO {
                    let _ = sender.unbounded_send(Command::SeekTo(seconds(position)));
                }
            });
            let run = mpris.run();
            let mut remote = Remote { mpris, track, last: None };
            let updates = async {
                while let Some(state) = states.next().await {
                    remote.publish(state).await;
                }
            };
            futures::future::join(run, updates).await;
        });
    });
    if let Err(e) = thread {
        eprintln!("MPRIS disabled: {e}");
    }
}

fn observe_player(active: &Entity<Player>, states: mpsc::UnboundedSender<State>, cx: &mut App) -> Subscription {
    let mut last = State::of(active.read(cx));
    let _ = states.unbounded_send(last.clone());
    cx.observe(active, move |entity, cx| {
        let state = State::of(entity.read(cx));
        if state.worth_sending(&last) {
            last = state.clone();
            let _ = states.unbounded_send(state);
        }
    })
}

pub(crate) fn init(cx: &mut App) {
    let (states, receiver) = mpsc::unbounded();
    let (sender, mut commands) = mpsc::unbounded();
    spawn_remote(receiver, sender);
    let active = player(cx);
    let mut binding = (active.entity_id(), observe_player(&active, states.clone(), cx));
    cx.observe_global::<PlayerHandle>(move |cx| {
        let active = player(cx);
        if binding.0 != active.entity_id() {
            binding = (active.entity_id(), observe_player(&active, states.clone(), cx));
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Some(command) = commands.next().await {
            cx.update(|cx| {
                player(cx).update(cx, |p, cx| {
                    if p.video.is_none() {
                        return;
                    }
                    match command {
                        Command::Play if p.eof => p.toggle_pause(cx),
                        Command::Play => p.set_paused(false, cx),
                        Command::Pause => p.set_paused(true, cx),
                        Command::PlayPause => p.toggle_pause(cx),
                        Command::Next => {
                            if let Some(index) = p.queue_pos.filter(|i| i + 1 < p.queue.len()) {
                                p.play_queue_index(index + 1, cx);
                            }
                        }
                        Command::Previous => {
                            if let Some(index) = p.queue_pos.filter(|i| *i > 0) {
                                p.play_queue_index(index - 1, cx);
                            }
                        }
                        Command::SeekBy(offset) => p.seek_by(offset),
                        Command::SeekTo(position) => p.seek_to(position),
                    }
                    cx.notify();
                })
            });
        }
    })
    .detach();
}
