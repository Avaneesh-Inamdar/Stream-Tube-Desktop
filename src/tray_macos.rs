//! macOS counterpart of the Windows `tray`: with "Minimize to tray" on, closing
//! the main window hides it and the Dock icon and leaves an icon in the menu bar
//! (Open / Quit), the way Windows keeps the app in the notification area.
use futures::{StreamExt, channel::mpsc};
use gpui_kit::{App, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSImage, NSMenu, NSMenuItem, NSStatusBar,
    NSStatusItem, NSVariableStatusItemLength, NSView, NSWindow,
};
use objc2_foundation::{NSData, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;

#[derive(Clone, Copy)]
enum Action {
    Open,
    Quit,
}

/// The hidden main window and the menu bar icon standing in for it.
struct Hidden {
    window: Retained<NSWindow>,
    item: Retained<NSStatusItem>,
    _target: Retained<Target>,
}

thread_local! {
    static HIDDEN: RefCell<Option<Hidden>> = const { RefCell::new(None) };
    static ACTIONS: RefCell<Option<mpsc::UnboundedSender<Action>>> = const { RefCell::new(None) };
}

fn send(action: Action) {
    ACTIONS.with_borrow(|a| {
        if let Some(a) = a {
            let _ = a.unbounded_send(action);
        }
    });
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and Target doesn't implement Drop
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "StreamTubeTrayTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(open:))]
        fn open(&self, _sender: Option<&AnyObject>) {
            send(Action::Open);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            send(Action::Quit);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: NSObject's init
        unsafe { msg_send![super(this), init] }
    }
}

fn ns_window(window: &Window) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: GPUI's window handle points at the live content view of the window
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    view.window()
}

fn menu_item(
    title: &str,
    action: objc2::runtime::Sel,
    target: &Target,
    mtm: MainThreadMarker,
) -> Retained<NSMenuItem> {
    let title = NSString::from_str(&crate::i18n::t(title));
    // SAFETY: the selector is one Target implements, and Target outlives the item (see Hidden)
    unsafe {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &title,
            Some(action),
            &NSString::new(),
        );
        item.setTarget(Some(target));
        item
    }
}

fn status_item(target: &Target, mtm: MainThreadMarker) -> Retained<NSStatusItem> {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        if let Some(icon) = NSImage::initWithData(
            NSImage::alloc(),
            &NSData::with_bytes(include_bytes!("../assets/logo.png")),
        ) {
            icon.setSize(NSSize::new(18.0, 18.0));
            button.setImage(Some(&icon));
        }
        button.setToolTip(Some(&NSString::from_str("Stream Tube")));
    }
    let menu = NSMenu::new(mtm);
    menu.addItem(&menu_item("Open", sel!(open:), target, mtm));
    menu.addItem(&menu_item("Quit", sel!(quit:), target, mtm));
    item.setMenu(Some(&menu));
    item
}

fn set_dock_icon(visible: bool, mtm: MainThreadMarker) {
    let policy = if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
}

/// Takes the menu bar icon away; the window stays as it is.
fn discard(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
    let hidden = HIDDEN.take()?;
    NSStatusBar::systemStatusBar().removeStatusItem(&hidden.item);
    set_dock_icon(true, mtm);
    Some(hidden.window)
}

fn show(window: &NSWindow, mtm: MainThreadMarker) {
    window.makeKeyAndOrderFront(None);
    #[allow(deprecated)] // `activate` needs macOS 14
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
}

/// Hides the main window to the menu bar when the setting asks for it.
pub(crate) fn hide(window: &Window) -> bool {
    if !crate::store::settings_snapshot().minimize_to_tray {
        return false;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    if HIDDEN.with_borrow(Option::is_some) {
        return true;
    }
    let Some(ns_window) = ns_window(window) else {
        return false;
    };
    let target = Target::new(mtm);
    let item = status_item(&target, mtm);
    ns_window.orderOut(None);
    set_dock_icon(false, mtm);
    HIDDEN.set(Some(Hidden {
        window: ns_window,
        item,
        _target: target,
    }));
    true
}

/// Shows the window again if it is the one hidden to the menu bar.
pub(crate) fn restore(window: &Window) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(ns_window) = ns_window(window) else {
        return false;
    };
    if !HIDDEN.with_borrow(|h| h.as_ref().is_some_and(|h| h.window == ns_window)) {
        return false;
    }
    discard(mtm);
    show(&ns_window, mtm);
    true
}

/// Clicking the app in Finder, Spotlight or Launchpad while it is hidden.
pub(crate) fn reopen(_: &mut App) {
    send(Action::Open);
}

pub(crate) fn init(cx: &mut App) {
    let (actions, mut received) = mpsc::unbounded();
    ACTIONS.set(Some(actions));
    cx.spawn(async move |cx| {
        while let Some(action) = received.next().await {
            let _ = cx.update(|cx| {
                let pip = crate::ui::pip::handle(cx);
                let Some(main) = cx.windows().into_iter().find(|w| Some(*w) != pip) else {
                    return;
                };
                let _ = main.update(cx, |_, window, _| match action {
                    Action::Open => {
                        crate::ui::app::restore_window(window);
                        window.activate_window();
                    }
                    Action::Quit => {
                        if let Some(mtm) = MainThreadMarker::new() {
                            discard(mtm);
                        }
                        // closes without asking `on_window_should_close`, so the app quits
                        window.remove_window();
                    }
                });
            });
        }
    })
    .detach();
}
