//! key-remap: swap the roles of ` and Alt+` on a US keyboard.
//!
//! - ` alone        -> toggle IME on/off (sends the Hankaku/Zenkaku key)
//! - Alt + `        -> type the ` character itself
//! - Shift + ` (~), Ctrl+`, Win+` etc. are left untouched.
//!
//! Implementation: a low-level keyboard hook (WH_KEYBOARD_LL) watches for
//! VK_OEM_3. Handled presses are swallowed and an action is posted to the
//! main thread, which performs it with SendInput. No polling, no timers:
//! the process idles in GetMessage and wakes only on key events.

#![windows_subsystem = "windows"]

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// ---- Win32 constants ----
const WH_KEYBOARD_LL: i32 = 13;
const HC_ACTION: i32 = 0;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_APP: u32 = 0x8000;
const MSG_TOGGLE_IME: u32 = WM_APP + 1;
const MSG_TYPE_BACKQUOTE: u32 = WM_APP + 2;

const VK_OEM_3: u32 = 0xC0; // ` key on the US layout
const VK_MENU: i32 = 0x12;
const VK_LMENU: i32 = 0xA4;
const VK_RMENU: i32 = 0xA5;
const VK_CONTROL: i32 = 0x11;
const VK_SHIFT: i32 = 0x10;
const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;
const VK_ZENKAKU: u16 = 0xF3; // Hankaku/Zenkaku virtual key
const SC_ZENKAKU: u16 = 0x29; // Hankaku/Zenkaku scan code

const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_UNICODE: u32 = 0x0004;
const ERROR_ALREADY_EXISTS: u32 = 183;

/// Marker in dwExtraInfo for events injected by this process.
const MAGIC: usize = 0x4B52_4D50; // "KRMP"

// ---- Win32 structures ----
#[repr(C)]
#[derive(Clone, Copy)]
struct KbdLlHookStruct {
    vk_code: u32,
    scan_code: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KeybdInput {
    w_vk: u16,
    w_scan: u16,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseInput {
    dx: i32,
    dy: i32,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
union InputUnion {
    ki: KeybdInput,
    mi: MouseInput,
}

#[repr(C)]
struct Input {
    input_type: u32,
    union: InputUnion,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Msg {
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: Point,
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

extern "system" {
    fn SetWindowsHookExW(id_hook: i32, lpfn: HookProc, hmod: *mut c_void, thread_id: u32) -> *mut c_void;
    fn CallNextHookEx(hhk: *mut c_void, code: i32, w_param: usize, l_param: isize) -> isize;
    fn UnhookWindowsHookEx(hhk: *mut c_void) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32) -> i32;
    fn PostThreadMessageW(thread_id: u32, msg: u32, w_param: usize, l_param: isize) -> i32;
    fn GetCurrentThreadId() -> u32;
    fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    fn SendInput(n: u32, inputs: *const Input, cb_size: i32) -> u32;
    fn GetAsyncKeyState(vkey: i32) -> i16;
    fn CreateMutexW(attrs: *mut c_void, initial_owner: i32, name: *const u16) -> *mut c_void;
    fn GetLastError() -> u32;
    fn GetModuleFileNameW(hmod: *mut c_void, buf: *mut u16, size: u32) -> u32;
    fn RegCreateKeyExW(hkey: *mut c_void, subkey: *const u16, reserved: u32, class: *const u16, options: u32, sam: u32, sa: *const c_void, out_key: *mut *mut c_void, disp: *mut u32) -> i32;
    fn RegOpenKeyExW(hkey: *mut c_void, subkey: *const u16, options: u32, sam: u32, out_key: *mut *mut c_void) -> i32;
    fn RegSetValueExW(hkey: *mut c_void, name: *const u16, reserved: u32, ty: u32, data: *const u8, cb: u32) -> i32;
    fn RegDeleteValueW(hkey: *mut c_void, name: *const u16) -> i32;
    fn RegCloseKey(hkey: *mut c_void) -> i32;
}

// ---- Logon auto-start (HKCU Run key) ----
const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001usize as *mut c_void;
const KEY_WRITE: u32 = 0x0002_0006;
const KEY_SET_VALUE_ONLY: u32 = 0x0002;
const REG_SZ: u32 = 1;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Register (or refresh) this exe in the HKCU Run key so it starts at logon.
/// Runs on every launch, so moving the exe updates the registered path.
fn ensure_autostart() {
    unsafe {
        let mut buf = [0u16; 1024];
        let n = GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32);
        if n == 0 {
            return;
        }
        let exe = String::from_utf16_lossy(&buf[..n as usize]);
        let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let name = wide("key-remap");
        let data = wide(&format!("\"{exe}\""));
        let mut hkey: *mut c_void = std::ptr::null_mut();
        if RegCreateKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, std::ptr::null(), 0, KEY_WRITE, std::ptr::null(), &mut hkey, std::ptr::null_mut()) == 0 {
            RegSetValueExW(hkey, name.as_ptr(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32);
            RegCloseKey(hkey);
        }
    }
}

/// Remove the Run key registration (`key-remap.exe --uninstall`).
fn remove_autostart() {
    unsafe {
        let subkey = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let name = wide("key-remap");
        let mut hkey: *mut c_void = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_SET_VALUE_ONLY, &mut hkey) == 0 {
            RegDeleteValueW(hkey, name.as_ptr());
            RegCloseKey(hkey);
        }
    }
}

// ---- Shared state ----
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);
/// The current ` press was swallowed by us, so its key-up must be swallowed too.
static SWALLOWED: AtomicBool = AtomicBool::new(false);
/// The IME toggle was already sent for this physical press (ignore auto-repeat).
static TOGGLE_SENT: AtomicBool = AtomicBool::new(false);

fn key_is_down(vk: i32) -> bool {
    (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
}

fn post(msg: u32) {
    unsafe {
        PostThreadMessageW(MAIN_THREAD.load(Ordering::SeqCst), msg, 0, 0);
    }
}

unsafe extern "system" fn hook_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code == HC_ACTION {
        let info = &*(l_param as *const KbdLlHookStruct);
        if info.extra_info != MAGIC && info.vk_code == VK_OEM_3 {
            let msg = w_param as u32;
            let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if is_down {
                let alt = key_is_down(VK_MENU);
                let ctrl = key_is_down(VK_CONTROL);
                let win = key_is_down(VK_LWIN) || key_is_down(VK_RWIN);
                if !ctrl && !win && alt {
                    // Alt+` -> type a literal backquote. Repeats keep typing.
                    post(MSG_TYPE_BACKQUOTE);
                    SWALLOWED.store(true, Ordering::SeqCst);
                    return 1;
                } else if !ctrl && !win && !alt && !key_is_down(VK_SHIFT) {
                    // Plain ` -> toggle IME, once per physical press.
                    if !TOGGLE_SENT.swap(true, Ordering::SeqCst) {
                        post(MSG_TOGGLE_IME);
                    }
                    SWALLOWED.store(true, Ordering::SeqCst);
                    return 1;
                }
            } else if is_up && SWALLOWED.swap(false, Ordering::SeqCst) {
                TOGGLE_SENT.store(false, Ordering::SeqCst);
                return 1;
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
}

// ---- Actions (run on the main thread) ----
fn key_input(vk: u16, scan: u16, flags: u32) -> Input {
    Input {
        input_type: INPUT_KEYBOARD,
        union: InputUnion {
            ki: KeybdInput { w_vk: vk, w_scan: scan, flags, time: 0, extra_info: MAGIC },
        },
    }
}

fn send_inputs(inputs: &[Input]) {
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<Input>() as i32);
    }
}

fn toggle_ime() {
    send_inputs(&[
        key_input(VK_ZENKAKU, SC_ZENKAKU, 0),
        key_input(VK_ZENKAKU, SC_ZENKAKU, KEYEVENTF_KEYUP),
    ]);
}

fn type_backquote() {
    // Release Alt while sending so applications do not treat the character
    // as an Alt shortcut, then restore it if it is still physically held.
    let lalt = key_is_down(VK_LMENU);
    let ralt = key_is_down(VK_RMENU);
    let mut batch: Vec<Input> = Vec::new();
    if lalt {
        batch.push(key_input(VK_LMENU as u16, 0, KEYEVENTF_KEYUP));
    }
    if ralt {
        batch.push(key_input(VK_RMENU as u16, 0, KEYEVENTF_KEYUP));
    }
    batch.push(key_input(0, 0x60, KEYEVENTF_UNICODE)); // '`'
    batch.push(key_input(0, 0x60, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    send_inputs(&batch);

    let mut restore: Vec<Input> = Vec::new();
    if lalt && key_is_down(VK_LMENU) {
        restore.push(key_input(VK_LMENU as u16, 0, 0));
    }
    if ralt && key_is_down(VK_RMENU) {
        restore.push(key_input(VK_RMENU as u16, 0, 0));
    }
    if !restore.is_empty() {
        send_inputs(&restore);
    }
}

fn main() {
    // `key-remap.exe --uninstall` removes the auto-start registration and exits.
    if std::env::args().skip(1).any(|a| a == "--uninstall") {
        remove_autostart();
        return;
    }
    unsafe {
        // Single instance.
        let mutex_name: Vec<u16> = "KeyRemapGraveImeMutex\0".encode_utf16().collect();
        CreateMutexW(std::ptr::null_mut(), 0, mutex_name.as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        // From the first launch on, start automatically at logon.
        ensure_autostart();

        MAIN_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
        let hmod = GetModuleHandleW(std::ptr::null());
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, hook_proc, hmod, 0);
        if hook.is_null() {
            return;
        }

        let mut msg: Msg = std::mem::zeroed();
        loop {
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r == 0 || r == -1 {
                break;
            }
            match msg.message {
                MSG_TOGGLE_IME => toggle_ime(),
                MSG_TYPE_BACKQUOTE => type_backquote(),
                _ => {}
            }
        }
        UnhookWindowsHookEx(hook);
    }
}
