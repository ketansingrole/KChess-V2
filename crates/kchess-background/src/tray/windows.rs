//! The notification area icon: `Shell_NotifyIconW` on a hidden window of the app's main thread,
//! whose messages GPUI's own loop delivers. A left click opens KChess; a right click shows the
//! menu at the pointer (`TrackPopupMenu`). When Explorer restarts it broadcasts `TaskbarCreated`,
//! and the icon is added again.

// Win32's window, notification icon and menu calls are unsafe, each with its SAFETY comment.
#![allow(
    unsafe_code,
    reason = "Win32's notification icon and popup menu calls are unsafe; this module owns the tray icon"
)]

use std::cell::RefCell;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    GetCursorPos, GetSystemMetrics, HICON, HMENU, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR,
    LoadIconW, LoadImageW, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, PostMessageW,
    RegisterClassW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WINDOW_EX_STYLE,
    WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::{Event, Events, Icon, Item};

/// The message the icon sends its window.
const CALLBACK: u32 = WM_APP + 0x4b43;
/// The icon's id on its window.
const ICON_ID: u32 = 1;

/// The shown icon's window, menu and callback, for the window procedure to reach.
struct State {
    window: HWND,
    menu: Vec<Item>,
    events: Events,
    /// `TaskbarCreated`: Explorer started again, without the icon.
    taskbar_created: u32,
    data: NOTIFYICONDATAW,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub struct Tray {
    window: HWND,
}

/// The app's own icon from the executable's resources (`build.rs` puts it at id 1), at the
/// tray's size; the system's application icon in a program without it (a test).
fn app_icon() -> HICON {
    // SAFETY: reads this process's own module and resources.
    unsafe {
        let size = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let own = GetModuleHandleW(None).ok().and_then(|module| {
            LoadImageW(
                Some(module.into()),
                PCWSTR(1 as _),
                IMAGE_ICON,
                size.0,
                size.1,
                LR_DEFAULTCOLOR,
            )
            .ok()
        });
        match own {
            Some(handle) => HICON(handle.0),
            None => LoadIconW(None, IDI_APPLICATION).unwrap_or_default(),
        }
    }
}

/// `text` as the tooltip's fixed buffer, cut to fit with its terminating zero.
fn tip(text: &str) -> [u16; 128] {
    let mut buffer = [0u16; 128];
    for (slot, unit) in buffer.iter_mut().zip(text.encode_utf16().take(127)) {
        *slot = unit;
    }
    buffer
}

impl Tray {
    pub fn new(
        tooltip: &str,
        _icon: &Icon,
        menu: Vec<Item>,
        events: Events,
    ) -> Result<Tray, String> {
        if STATE.with_borrow(Option::is_some) {
            return Err("The tray icon is already shown.".into());
        }
        // SAFETY: a window class and hidden window of this thread, with this module's procedure.
        let window = unsafe {
            let instance = GetModuleHandleW(None).map_err(|cause| cause.message())?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: w!("KChessTray"),
                ..WNDCLASSW::default()
            };
            // Registered once per process; a second registration fails harmlessly.
            RegisterClassW(&class);
            // A hidden top-level window rather than a message-only one: only top-level windows
            // receive Explorer's `TaskbarCreated` broadcast.
            CreateWindowExW(
                WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0),
                w!("KChessTray"),
                w!("KChess"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )
            .map_err(|cause| cause.message())?
        };
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: CALLBACK,
            hIcon: app_icon(),
            szTip: tip(tooltip),
            ..NOTIFYICONDATAW::default()
        };
        // SAFETY: a static string.
        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
        // SAFETY: `data` names this thread's window and lives through the call.
        let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool();
        if !added {
            // SAFETY: the window made above, on this thread.
            unsafe { DestroyWindow(window) }.ok();
            return Err("The notification area did not take the icon.".into());
        }
        STATE.set(Some(State {
            window,
            menu,
            events,
            taskbar_created,
            data,
        }));
        Ok(Tray { window })
    }

    pub fn set_menu(&mut self, menu: Vec<Item>) {
        STATE.with_borrow_mut(|state| {
            if let Some(state) = state.as_mut().filter(|state| state.window == self.window) {
                state.menu = menu;
            }
        });
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let state = STATE.take();
        if let Some(state) = &state {
            // SAFETY: the icon added by `new`, on this thread's window.
            // Already gone when Explorer restarted without it.
            let _removed = unsafe { Shell_NotifyIconW(NIM_DELETE, &state.data) };
        }
        // SAFETY: the window made by `new`, on this thread.
        unsafe { DestroyWindow(self.window) }.ok();
    }
}

/// A popup menu of `items`; ids are the items' own.
fn build(items: &[Item]) -> windows::core::Result<HMENU> {
    // SAFETY: a new menu, filled with strings that outlive each call.
    unsafe {
        let menu = CreatePopupMenu()?;
        for item in items {
            match item {
                Item::Separator => AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null())?,
                Item::Submenu { label, items } => {
                    let sub = build(items)?;
                    AppendMenuW(
                        menu,
                        MF_POPUP | MF_STRING,
                        sub.0 as usize,
                        &HSTRING::from(menu_text(label)),
                    )?;
                }
                Item::Action {
                    id,
                    label,
                    enabled,
                    checked,
                } => {
                    let mut flags = MF_STRING;
                    if !enabled {
                        flags |= MF_GRAYED;
                    }
                    if *checked == Some(true) {
                        flags |= MF_CHECKED;
                    }
                    AppendMenuW(menu, flags, *id as usize, &HSTRING::from(menu_text(label)))?;
                }
            }
        }
        Ok(menu)
    }
}

/// A label as a Win32 menu shows it: `&` marks the access key, so a written one is doubled.
fn menu_text(label: &str) -> String {
    label.replace('&', "&&")
}

/// Show the menu at the pointer and report what was picked.
fn show_menu(window: HWND, items: &[Item], events: &Events) {
    let Ok(menu) = build(items) else {
        return;
    };
    let mut point = POINT::default();
    // SAFETY: this thread's window and menu; the menu is destroyed (with its submenus) after.
    let picked = unsafe {
        GetCursorPos(&mut point).ok();
        // The menu closes when the person clicks elsewhere only while its window is in front.
        let _ = SetForegroundWindow(window);
        let picked = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            point.x,
            point.y,
            None,
            window,
            None,
        );
        // As the documentation asks, so a second right click opens the menu again.
        PostMessageW(Some(window), WM_NULL, WPARAM(0), LPARAM(0)).ok();
        DestroyMenu(menu).ok();
        picked.0
    };
    if let Ok(id) = u32::try_from(picked)
        && id != 0
    {
        events(Event::Chose(id));
    }
}

extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let act = STATE.with_borrow(|state| {
        let state = state.as_ref().filter(|state| state.window == window)?;
        if message == CALLBACK {
            return match lparam.0 as u32 {
                WM_LBUTTONUP => Some(Act::Open(state.events.clone())),
                WM_RBUTTONUP | WM_CONTEXTMENU => {
                    Some(Act::Menu(state.menu.clone(), state.events.clone()))
                }
                _ => None,
            };
        }
        (message == state.taskbar_created).then(|| Act::Readd(Box::new(state.data)))
    });
    // Acted on with the state released: an event may change the menu.
    match act {
        Some(Act::Open(events)) => {
            events(Event::Open);
            LRESULT(0)
        }
        Some(Act::Menu(items, events)) => {
            show_menu(window, &items, &events);
            LRESULT(0)
        }
        Some(Act::Readd(data)) => {
            // SAFETY: the icon's own data, on its window.
            unsafe {
                if !Shell_NotifyIconW(NIM_MODIFY, &*data).as_bool() {
                    // Not added again only when the notification area refuses it once more.
                    let _added = Shell_NotifyIconW(NIM_ADD, &*data);
                }
            }
            LRESULT(0)
        }
        // SAFETY: the default handling of a message this window was sent.
        None => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

enum Act {
    Open(Events),
    Menu(Vec<Item>, Events),
    Readd(Box<NOTIFYICONDATAW>),
}

#[cfg(test)]
mod tests {
    #[test]
    fn labels_and_tooltips_fit_win32() {
        assert_eq!(super::menu_text("Q&A"), "Q&&A");
        let long = "x".repeat(300);
        let tip = super::tip(&long);
        assert_eq!(tip[126], u16::from(b'x'));
        assert_eq!(tip[127], 0);
    }
}
