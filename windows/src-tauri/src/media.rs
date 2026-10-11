// Windows media & desktop integrations: Spotify and WhatsApp.
//
// Spotify:
// - Uses Windows Runtime (WinRT) GlobalSystemMediaTransportControlsSessionManager (GSMTC)
//   to detect track metadata, position, duration, playing status, repeat, and shuffle.
// - Supports both Spotify Desktop and Spotify Windows Store (UWP).
// - Supports direct GSMTC control (Play, Pause, Toggle, Next, Prev, Seek, Shuffle, Repeat)
//   with fallback to Win32 media keys and window title scanner.
//
// WhatsApp:
// - Without API keys or login: detects browser tabs ("(N) WhatsApp") or desktop app
//   windows, extracting unread message counts.

use serde::Serialize;
use windows::core::BOOL;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Media::MediaPlaybackAutoRepeatMode;
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
    pub album: String,
    pub duration: f64,
    pub position: f64,
    pub shuffle: bool,
    pub repeat: bool,
}

/// Finds the active Spotify GSMTC session if available.
fn spotify_session() -> Option<GlobalSystemMediaTransportControlsSession> {
    let mgr_async = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().ok()?;
    let mgr = mgr_async.get().ok()?;
    let sessions = mgr.GetSessions().ok()?;

    for session in sessions {
        let app_id = session.SourceAppUserModelId().unwrap_or_default().to_string();
        if app_id.to_lowercase().contains("spotify") {
            return Some(session);
        }
    }
    None
}

/// Reads current playback details from Windows Media Transport Controls (GSMTC).
fn now_playing_gsmtc() -> Option<NowPlaying> {
    let session = spotify_session()?;

    let playback_info = session.GetPlaybackInfo().ok();
    let status = playback_info
        .as_ref()
        .and_then(|info| info.PlaybackStatus().ok())
        .unwrap_or(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Closed);

    let playing = status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

    let shuffle = playback_info
        .as_ref()
        .and_then(|info| info.IsShuffleActive().ok())
        .and_then(|ref_bool| ref_bool.Value().ok())
        .unwrap_or(false);

    let repeat = playback_info
        .as_ref()
        .and_then(|info| info.AutoRepeatMode().ok())
        .and_then(|ref_mode| ref_mode.Value().ok())
        .map(|m| m != MediaPlaybackAutoRepeatMode::None)
        .unwrap_or(false);

    let media_async = session.TryGetMediaPropertiesAsync().ok()?;
    let media = media_async.get().ok()?;

    let artist = media.Artist().unwrap_or_default().to_string();
    let title = media.Title().unwrap_or_default().to_string();
    let album = media.AlbumTitle().unwrap_or_default().to_string();

    let mut position = 0.0f64;
    let mut duration = 0.0f64;

    if let Ok(timeline) = session.GetTimelineProperties() {
        if let Ok(pos_ts) = timeline.Position() {
            position = (pos_ts.Duration.max(0) as f64) / 10_000_000.0;
        }
        if let Ok(end_ts) = timeline.EndTime() {
            duration = (end_ts.Duration.max(0) as f64) / 10_000_000.0;
        }
    }

    Some(NowPlaying {
        running: true,
        playing,
        artist,
        title,
        album,
        duration,
        position,
        shuffle,
        repeat,
    })
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

/// Fallback: Polls the Spotify desktop window on Windows via legacy window title.
fn now_playing_win32() -> NowPlaying {
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
            ..Default::default()
        };
    }

    if title == "Advertisement" {
        return NowPlaying {
            running: true,
            playing: true,
            artist: "Spotify".to_string(),
            title: "Advertisement".to_string(),
            ..Default::default()
        };
    }

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
                    ..Default::default()
                };
            }
        }
    }

    NowPlaying {
        running: true,
        playing: true,
        artist: String::new(),
        title,
        ..Default::default()
    }
}

/// Polls what is currently playing on Spotify.
pub fn now_playing() -> NowPlaying {
    if let Some(np) = now_playing_gsmtc() {
        return np;
    }
    now_playing_win32()
}

/// Controls Spotify via WinRT GSMTC commands where available, falling back to Media Keys.
pub fn control(action: &str, value: Option<f64>) -> bool {
    if let Some(session) = spotify_session() {
        match action {
            "playPause" | "playpause" | "toggle" => {
                if let Ok(op) = session.TryTogglePlayPauseAsync() {
                    if let Ok(res) = op.get() {
                        return res;
                    }
                }
            }
            "play" => {
                if let Ok(op) = session.TryPlayAsync() {
                    if let Ok(res) = op.get() {
                        return res;
                    }
                }
            }
            "pause" => {
                if let Ok(op) = session.TryPauseAsync() {
                    if let Ok(res) = op.get() {
                        return res;
                    }
                }
            }
            "next" => {
                if let Ok(op) = session.TrySkipNextAsync() {
                    if let Ok(res) = op.get() {
                        return res;
                    }
                }
            }
            "prev" | "previous" => {
                if let Ok(op) = session.TrySkipPreviousAsync() {
                    if let Ok(res) = op.get() {
                        return res;
                    }
                }
            }
            "seek" => {
                if let Some(secs) = value {
                    let ticks = (secs.max(0.0) * 10_000_000.0) as i64;
                    if let Ok(op) = session.TryChangePlaybackPositionAsync(ticks) {
                        if let Ok(res) = op.get() {
                            return res;
                        }
                    }
                }
            }
            "shuffle" => {
                let on = value.unwrap_or(0.0) > 0.5;
                if let Ok(op) = session.TryChangeShuffleActiveAsync(on) {
                    if let Ok(res) = op.get() {
                        return res;
                    }
                }
            }
            "repeat" => {
                let on = value.unwrap_or(0.0) > 0.5;
                let mode = if on {
                    MediaPlaybackAutoRepeatMode::Track
                } else {
                    MediaPlaybackAutoRepeatMode::None
                };
                if let Ok(op) = session.TryChangeAutoRepeatModeAsync(mode) {
                    if let Ok(res) = op.get() {
                        return res;
                    }
                }
            }
            _ => {}
        }
    }

    // Fallback to media keys
    press(action).is_ok()
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

/// Checks if Spotify is installed (checks %APPDATA%\Spotify\Spotify.exe, WindowsApps or PATH).
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
        // Microsoft Store Spotify package check
        let store_path = std::path::Path::new(&localappdata)
            .join("Packages");
        if let Ok(entries) = std::fs::read_dir(store_path) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().to_lowercase().contains("spotify") {
                    return true;
                }
            }
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
