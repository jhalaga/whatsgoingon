use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::process::Command;
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, BOOL, HANDLE, HWND, LPARAM, RECT};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetExtendedTcpTable, MIB_TCP6TABLE_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
    TCP_TABLE_OWNER_PID_LISTENER,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::Console::{
    AttachConsole, FreeConsole, GetConsoleScreenBufferInfo, GetStdHandle,
    ReadConsoleOutputCharacterW, CONSOLE_SCREEN_BUFFER_INFO, COORD, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, TerminateProcess, PROCESS_TERMINATE,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, PostMessageW, SetForegroundWindow, ShowWindow, SW_HIDE, SW_MINIMIZE,
    SW_RESTORE, SW_SHOW, WM_CLOSE,
};

use crate::model::{ConsolePeekResult, WindowInfo};

static CONSOLE_ATTACH_LOCK: Mutex<()> = Mutex::new(());

/// Enumerate all top-level windows and group them by owning PID.
pub fn enumerate_windows() -> HashMap<u32, Vec<WindowInfo>> {
    let mut map: HashMap<u32, Vec<WindowInfo>> = HashMap::new();
    let ptr = &mut map as *mut HashMap<u32, Vec<WindowInfo>> as LPARAM;

    unsafe {
        EnumWindows(Some(enum_windows_proc), ptr);
    }

    map
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let map = unsafe { &mut *(lparam as *mut HashMap<u32, Vec<WindowInfo>>) };

    let mut pid: u32 = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    if pid == 0 {
        return 1;
    }

    let is_visible = unsafe { IsWindowVisible(hwnd) != 0 };
    let is_minimized = unsafe { IsIconic(hwnd) != 0 };

    let title = unsafe { get_window_text(hwnd) };
    let class_name = unsafe { get_window_class(hwnd) };

    // Skip invisible helper/IME/MSCTFIME windows unless they look like console/terminal windows
    if !is_visible && !is_console_or_terminal_class(&class_name) && title.is_empty() {
        return 1;
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        GetWindowRect(hwnd, &mut rect);
    }

    let info = WindowInfo {
        hwnd: hwnd as isize,
        owner_pid: pid,
        title,
        class_name,
        is_visible,
        is_minimized,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    };

    map.entry(pid).or_default().push(info);
    1
}

fn is_console_or_terminal_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "ConsoleWindowClass"
            | "CASCADIA_HOSTING_WINDOW_CLASS"
            | "PseudoConsoleWindow"
            | "mintty"
            | "VirtualConsoleClass"
            | "org.wezfurlong.wezterm"
            | "Window Class"
    )
}

unsafe fn get_window_text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if copied <= 0 {
        return String::new();
    }
    OsString::from_wide(&buf[..copied as usize])
        .to_string_lossy()
        .into_owned()
}

unsafe fn get_window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let copied = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if copied <= 0 {
        return String::new();
    }
    OsString::from_wide(&buf[..copied as usize])
        .to_string_lossy()
        .into_owned()
}

/// Reliably bring a window to the foreground, even when our app is currently focused.
pub fn focus_window(hwnd_val: isize) -> Result<(), String> {
    let hwnd = hwnd_val as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err("Window no longer exists.".to_string());
        }

        if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOW);
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }

        let fg = GetForegroundWindow();
        let fg_thread = if !fg.is_null() {
            GetWindowThreadProcessId(fg, std::ptr::null_mut())
        } else {
            0
        };
        let cur_thread = GetCurrentThreadId();
        let target_thread = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());

        if fg_thread != 0 && fg_thread != cur_thread {
            AttachThreadInput(cur_thread, fg_thread, 1);
        }
        if target_thread != 0 && target_thread != cur_thread {
            AttachThreadInput(cur_thread, target_thread, 1);
        }

        BringWindowToTop(hwnd);
        SetForegroundWindow(hwnd);

        if fg_thread != 0 && fg_thread != cur_thread {
            AttachThreadInput(cur_thread, fg_thread, 0);
        }
        if target_thread != 0 && target_thread != cur_thread {
            AttachThreadInput(cur_thread, target_thread, 0);
        }
    }
    Ok(())
}

/// Show or hide a window by HWND.
pub fn set_window_visible(hwnd_val: isize, visible: bool) -> Result<(), String> {
    let hwnd = hwnd_val as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err("Window no longer exists.".to_string());
        }
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE });
        if visible {
            focus_window(hwnd_val)?;
        }
    }
    Ok(())
}

/// Minimize a window by HWND.
pub fn minimize_window(hwnd_val: isize) -> Result<(), String> {
    let hwnd = hwnd_val as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err("Window no longer exists.".to_string());
        }
        ShowWindow(hwnd, SW_MINIMIZE);
    }
    Ok(())
}

/// Gracefully close a single window by sending WM_CLOSE (safe for multi-window hosts like Windows Terminal).
pub fn close_window(hwnd_val: isize) -> Result<(), String> {
    let hwnd = hwnd_val as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            return Err("Window no longer exists.".to_string());
        }
        if PostMessageW(hwnd, WM_CLOSE, 0, 0) == 0 {
            let err = GetLastError();
            return Err(format!("Failed to close window (Win32 error {})", err));
        }
    }
    Ok(())
}

/// Terminate a single process by PID.
pub fn terminate_process(pid: u32) -> Result<(), String> {
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            let err = GetLastError();
            return Err(format!(
                "Cannot open PID {} (Win32 error {} — may require Administrator privileges)",
                pid, err
            ));
        }
        let ok = TerminateProcess(handle, 1);
        CloseHandle(handle);
        if ok == 0 {
            let err = GetLastError();
            return Err(format!("Failed to terminate PID {} (error {})", pid, err));
        }
    }
    Ok(())
}

/// Terminate a list of PIDs (children first, then root).
pub fn terminate_process_tree(pids: &[u32]) -> (usize, Vec<String>) {
    let mut killed = 0usize;
    let mut errors = Vec::new();
    for &pid in pids {
        match terminate_process(pid) {
            Ok(()) => killed += 1,
            Err(e) => errors.push(e),
        }
    }
    (killed, errors)
}

/// Open a directory or select a file in Windows Explorer.
pub fn open_in_explorer(path: &str, select_file: bool) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("Path is empty.".to_string());
    }
    let mut cmd = Command::new("explorer.exe");
    if select_file {
        cmd.arg(format!("/select,{}", path));
    } else {
        cmd.arg(path);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to launch Explorer: {}", e))
}

/// Query all TCP listening ports grouped by owning PID (IPv4 + IPv6).
pub fn get_listening_tcp_ports() -> HashMap<u32, Vec<u16>> {
    let mut result: HashMap<u32, Vec<u16>> = HashMap::new();
    collect_tcp_v4(&mut result);
    collect_tcp_v6(&mut result);
    for ports in result.values_mut() {
        ports.sort_unstable();
        ports.dedup();
    }
    result
}

fn collect_tcp_v4(result: &mut HashMap<u32, Vec<u16>>) {
    unsafe {
        let mut size: u32 = 0;
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if size == 0 {
            return;
        }
        let mut buf = vec![0u8; size as usize];
        let ret = GetExtendedTcpTable(
            buf.as_mut_ptr() as *mut _,
            &mut size,
            0,
            AF_INET as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if ret != 0 {
            return;
        }
        let table = &*(buf.as_ptr() as *const MIB_TCPTABLE_OWNER_PID);
        let count = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        let rows = std::slice::from_raw_parts(rows_ptr, count);
        for row in rows {
            let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
            if port > 0 && row.dwOwningPid > 0 {
                result.entry(row.dwOwningPid).or_default().push(port);
            }
        }
    }
}

fn collect_tcp_v6(result: &mut HashMap<u32, Vec<u16>>) {
    unsafe {
        let mut size: u32 = 0;
        GetExtendedTcpTable(
            std::ptr::null_mut(),
            &mut size,
            0,
            AF_INET6 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if size == 0 {
            return;
        }
        let mut buf = vec![0u8; size as usize];
        let ret = GetExtendedTcpTable(
            buf.as_mut_ptr() as *mut _,
            &mut size,
            0,
            AF_INET6 as u32,
            TCP_TABLE_OWNER_PID_LISTENER,
            0,
        );
        if ret != 0 {
            return;
        }
        let table = &*(buf.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID);
        let count = table.dwNumEntries as usize;
        let rows_ptr = table.table.as_ptr();
        let rows = std::slice::from_raw_parts(rows_ptr, count);
        for row in rows {
            let port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
            if port > 0 && row.dwOwningPid > 0 {
                result.entry(row.dwOwningPid).or_default().push(port);
            }
        }
    }
}

/// Attach temporarily to a target process's Win32 console and read the visible/recent lines
/// from its screen buffer, then detach cleanly.
pub fn peek_console_output(pid: u32) -> ConsolePeekResult {
    let _guard = match CONSOLE_ATTACH_LOCK.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };

    unsafe {
        // Ensure we are detached from any console first
        FreeConsole();

        if AttachConsole(pid) == 0 {
            let err = GetLastError();
            let reason = match err {
                5 => "Access denied (process may be running as Administrator).".to_string(),
                6 => "No Win32 console attached (likely ConPTY / piped headless process).".to_string(),
                31 => "Process has no active console screen buffer.".to_string(),
                87 => "Process no longer exists.".to_string(),
                _ => format!("Cannot attach to console (Win32 error {}).", err),
            };
            return ConsolePeekResult::Unavailable { pid, reason };
        }

        let result = read_attached_console(pid);
        FreeConsole();
        result
    }
}

unsafe fn read_attached_console(pid: u32) -> ConsolePeekResult {
    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() || handle == (-1isize as HANDLE) {
        return ConsolePeekResult::Unavailable {
            pid,
            reason: "Console stdout handle is unavailable.".to_string(),
        };
    }

    let mut csbi: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    if unsafe { GetConsoleScreenBufferInfo(handle, &mut csbi) } == 0 {
        return ConsolePeekResult::Unavailable {
            pid,
            reason: "Could not read console screen buffer info (stdout may be redirected to a pipe)."
                .to_string(),
        };
    }

    let width = csbi.dwSize.X.max(1);
    let height = csbi.dwSize.Y.max(1);
    let cursor_y = csbi.dwCursorPosition.Y.clamp(0, height - 1);
    let bottom_row = csbi.srWindow.Bottom.max(cursor_y).clamp(0, height - 1);

    let max_lines = 45i16;
    let start_row = (bottom_row - max_lines + 1).max(0);
    let mut lines = Vec::new();
    let mut row_buf = vec![0u16; width as usize];

    for y in start_row..=bottom_row {
        let mut chars_read: u32 = 0;
        let coord = COORD { X: 0, Y: y };
        let ok = unsafe {
            ReadConsoleOutputCharacterW(
                handle,
                row_buf.as_mut_ptr(),
                width as u32,
                coord,
                &mut chars_read,
            )
        };
        if ok != 0 && chars_read > 0 {
            let line = OsString::from_wide(&row_buf[..chars_read as usize])
                .to_string_lossy()
                .trim_end()
                .to_string();
            lines.push(line);
        } else {
            lines.push(String::new());
        }
    }

    // Trim trailing empty lines
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    // Trim leading empty lines
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }

    let is_blank = lines.is_empty();
    ConsolePeekResult::Success {
        pid,
        lines,
        is_blank,
        buffer_cols: width,
        buffer_rows: height,
    }
}
