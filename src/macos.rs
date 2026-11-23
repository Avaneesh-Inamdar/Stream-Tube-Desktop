use gpui_kit::*;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowButton, NSWindowStyleMask};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::OsString;
use std::path::Path;

pub fn homebrew_prefix() -> &'static Path {
    Path::new(if cfg!(target_arch = "aarch64") { "/opt/homebrew" } else { "/usr/local" })
}

/// Finder launches do not inherit the shell's Homebrew paths.
pub fn path_with_homebrew() -> Option<OsString> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut paths: Vec<_> = std::env::split_paths(&path).filter(|dir| !dir.as_os_str().is_empty()).collect();
    let bin = homebrew_prefix().join("bin");
    if paths.contains(&bin) || !bin.is_dir() {
        return None;
    }
    paths.push(bin);
    std::env::join_paths(paths).ok()
}

pub fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return None };
    // SAFETY: GPUI's live AppKit window handle points to its NSView on the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

pub fn configure_pip_window(window: &Window) {
    if let Some(window) = native_window(window) {
        // GPUI omits the resize style for titlebar-less macOS windows.
        window.setStyleMask(window.styleMask() | NSWindowStyleMask::Resizable);
        if let Some(zoom) = window.standardWindowButton(NSWindowButton::ZoomButton) {
            zoom.setHidden(true);
        }
        window.setHidesOnDeactivate(false);
    }
}

actions!(streamtube, [Quit]);

pub fn init_menu(main_window: AnyWindowHandle, cx: &mut App) {
    cx.on_action(move |_: &Quit, cx| {
        let _ = main_window.update(cx, |_, window, cx| {
            crate::save_window_bounds(window);
            crate::player::controller::player(cx).update(cx, |p, _| p.save_progress(true));
        });
        cx.quit();
    });
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    cx.set_menus([Menu::new("Stream Tube").items([MenuItem::action("Quit Stream Tube", Quit)])]);
}
