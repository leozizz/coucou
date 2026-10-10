// Windows media & desktop integrations: Spotify and WhatsApp.
//
// Spotify:
// - Without API keys or OAuth: reads window title ("Artist - Track") of
//   SpotifyMainWindow.
// - Media controls: uses Win32 SendInput with virtual media keys (VK_MEDIA_*).
//
// WhatsApp:
// - Without API keys or login: detects browser tabs ("(N) WhatsApp") or desktop app
//   windows, extracting unread message counts.

use serde::Serialize;
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextW, IsWindowVisible, SetForegroundWindow, ShowWindow,
    SW_RESTORE,
};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NowPlaying {
    pub running: bool,
    pub playing: bool,
    pub artist: String,
    pub title: String,
}

struct SpotifyFound {
    hwnd: Option<HWND>,
    title: Option<String>,
}

unsafe extern "system" fn spotify_enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let found = unsafe { &mut *(lparam.0 as *mut SpotifyFound) };
    if found.hwnd.is_some() {
        return false.into();
    }
    // We do not check IsWindowVisible because minimized or background Spotify
    // windows still exist and report title correctly.
    let mut class = [0u16; 64];
    let class_len = unsafe { GetClassNameW(hwnd, &mut class) };
    if class_len == 0 {
        return true.into();
    }
    let class_name = String::from_utf16_lossy(&class[..class_len as usize]);
    if class_name == "SpotifyMainWindow" {
        found.hwnd = Some(hwnd);
        let mut buf = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if len > 0 {
            found.title = Some(String::from_utf16_lossy(&buf[..len as usize]));
        }
        return false.into();
    }
    true.into()
}

/// Polls the Spotify desktop window on Windows.
pub fn now_playing() -> NowPlaying {
    let mut found = SpotifyFound { hwnd: None, title: None };
    unsafe {
        let _ = EnumWindows(
            Some(spotify_enum_proc),
            LPARAM(&mut found as *mut SpotifyFound as isize),
        );
    };

    let Some(_hwnd) = found.hwnd else {
        return NowPlaying::default();
    };

    let title = found.title.unwrap_or_default();
    if title.is_empty() || title == "Spotify" {
        return NowPlaying {
            running: true,
            playing: false,
            artist: String::new(),
            title: String::new(),
        };
    }

    if title == "Advertisement" {
        return NowPlaying {
            running: true,
            playing: true,
            artist: "Spotify".to_string(),
            title: "Advertisement".to_string(),
        };
    }

    // "Artist - Title" or "Artist – Title"
    for sep in [" - ", " – "] {
        if let Some((artist, track)) = title.split_once(sep) {
            let artist = artist.trim();
            let track = track.trim();
            if !artist.is_empty() && !track.is_empty() {
                return NowPlaying {
                    running: true,
                    playing: true,
                    artist: artist.to_string(),
                    title: track.to_string(),
                };
            }
        }
    }

    // Window has a custom title but not standard separator
    NowPlaying {
        running: true,
        playing: true,
        artist: String::new(),
        title,
    }
}

/// Sends global Windows media key events (play/pause, next, previous).
pub fn press(action: &str) -> Result<(), String> {
    let vk = match action {
        "playPause" | "playpause" | "toggle" | "play" | "pause" => 0xB3u16, // VK_MEDIA_PLAY_PAUSE
        "next" => 0xB0u16,                                                 // VK_MEDIA_NEXT_TRACK
        "prev" | "previous" => 0xB1u16,                                    // VK_MEDIA_PREV_TRACK
        _ => return Err("unknown media key".into()),
    };
    let mk = |up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let sent = unsafe { SendInput(&[mk(false), mk(true)], std::mem::size_of::<INPUT>() as i32) };
    if sent == 2 {
        Ok(())
    } else {
        Err("media key not delivered".into())
    }
}

/// Brings Spotify to front or launches `spotify:`.
pub fn open_spotify() -> bool {
    let mut found = SpotifyFound { hwnd: None, title: None };
    unsafe {
        let _ = EnumWindows(
            Some(spotify_enum_proc),
            LPARAM(&mut found as *mut SpotifyFound as isize),
        );
    };

    if let Some(hwnd) = found.hwnd {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return true;
    }

    // Try starting via URL protocol `spotify:`
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "spotify:"])
        .spawn();
    true
}

/// Checks if Spotify is installed (checks %APPDATA%\Spotify\Spotify.exe or PATH).
pub fn spotify_installed() -> bool {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let path = std::path::Path::new(&appdata).join("Spotify").join("Spotify.exe");
        if path.exists() {
            return true;
        }
    }
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        let path = std::path::Path::new(&localappdata)
            .join("Microsoft")
            .join("WindowsApps")
            .join("Spotify.exe");
        if path.exists() {
            return true;
        }
    }
    crate::platform::find_on_path("spotify").is_some()
}

// ── WhatsApp (Window title unread counter) ───────────────────────────────────

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WhatsAppState {
    pub present: bool,
    pub unread: u32,
}

struct WaFound {
    present: bool,
    unread: u32,
}

unsafe extern "system" fn wa_enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let found = unsafe { &mut *(lparam.0 as *mut WaFound) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
        let mut buf = [0u16; 512];
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if len > 0 {
            let title = String::from_utf16_lossy(&buf[..len as usize]);
            let lower = title.to_lowercase();
            if lower.contains("whatsapp") {
                found.present = true;
                // e.g. "(3) WhatsApp" -> 3. Take the max unread count across all windows.
                let trimmed = title.trim_start();
                if let Some(rest) = trimmed.strip_prefix('(') {
                    if let Some(end) = rest.find(')') {
                        if let Ok(n) = rest[..end].trim().parse::<u32>() {
                            found.unread = found.unread.max(n);
                        }
                    }
                }
            }
        }
    }
    true.into()
}

/// Scans for visible WhatsApp windows/tabs and counts unread messages.
pub fn whatsapp() -> WhatsAppState {
    let mut found = WaFound { present: false, unread: 0 };
    unsafe {
        let _ = EnumWindows(
            Some(wa_enum_proc),
            LPARAM(&mut found as *mut WaFound as isize),
        );
    };
    WhatsAppState {
        present: found.present,
        unread: found.unread,
    }
}
