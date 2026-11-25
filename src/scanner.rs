use std::collections::{HashMap, HashSet};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::model::{
    AncestorInfo, ChildProcessInfo, OwnerCategory, PurposeTag, TerminalInstance, VisibilityState,
    WindowInfo,
};
use crate::win32;

const SHELL_AND_TERMINAL_EXES: &[&str] = &[
    "cmd.exe",
    "powershell.exe",
    "pwsh.exe",
    "windowsterminal.exe",
    "wt.exe",
    "openconsole.exe",
    "conhost.exe",
    "bash.exe",
    "sh.exe",
    "zsh.exe",
    "fish.exe",
    "nu.exe",
    "wsl.exe",
    "wslhost.exe",
    "mintty.exe",
    "git-bash.exe",
    "alacritty.exe",
    "wezterm-gui.exe",
    "hyper.exe",
    "tabby.exe",
    "warp.exe",
    "kitty.exe",
    "conemu64.exe",
    "conemu.exe",
    "cmder.exe",
    "mobaxterm.exe",
];

const CONSOLE_HELPER_EXES: &[&str] = &["conhost.exe", "openconsole.exe"];

const TERMINAL_HOST_EXES: &[&str] = &[
    "windowsterminal.exe",
    "wt.exe",
    "wezterm-gui.exe",
    "alacritty.exe",
    "mintty.exe",
    "hyper.exe",
    "tabby.exe",
    "warp.exe",
    "kitty.exe",
    "conemu64.exe",
    "conemu.exe",
    "cmder.exe",
];

const DEV_RUNTIME_EXES: &[&str] = &[
    "bun.exe",
    "node.exe",
    "deno.exe",
    "python.exe",
    "python3.exe",
    "pythonw.exe",
    "py.exe",
    "uvicorn.exe",
    "cargo.exe",
    "go.exe",
    "java.exe",
    "javaw.exe",
    "dotnet.exe",
    "ruby.exe",
    "php.exe",
    "rustc.exe",
    "esbuild.exe",
    "vite.exe",
    "tsx.exe",
    "caddy.exe",
    "nginx.exe",
    "cloudflared.exe",
    "ngrok.exe",
    "ssh.exe",
];

pub struct ProcessScanner {
    system: System,
    self_pid: u32,
}

#[derive(Clone)]
struct RawProc {
    pid: u32,
    parent_pid: Option<u32>,
    name: String,
    name_lower: String,
    exe_path: String,
    cwd: String,
    cmdline: String,
    cpu_percent: f32,
    memory_bytes: u64,
    start_time_secs: u64,
}

impl ProcessScanner {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet),
        );
        Self {
            system,
            self_pid: std::process::id(),
        }
    }

    pub fn scan(&mut self) -> Vec<TerminalInstance> {
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .with_cmd(UpdateKind::OnlyIfNotSet)
                .with_exe(UpdateKind::OnlyIfNotSet)
                .with_cwd(UpdateKind::OnlyIfNotSet),
        );

        let windows_by_pid = win32::enumerate_windows();
        let ports_by_pid = win32::get_listening_tcp_ports();

        let mut procs: HashMap<u32, RawProc> = HashMap::new();
        let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();

        for (pid, proc_info) in self.system.processes() {
            let pid_u32 = pid.as_u32();
            let parent_pid = proc_info.parent().map(Pid::as_u32);
            let name = proc_info.name().to_string_lossy().into_owned();
            let name_lower = name.to_lowercase();
            let exe_path = proc_info
                .exe()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let cwd = proc_info
                .cwd()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let cmdline = proc_info
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");

            if let Some(ppid) = parent_pid {
                children_map.entry(ppid).or_default().push(pid_u32);
            }

            procs.insert(
                pid_u32,
                RawProc {
                    pid: pid_u32,
                    parent_pid,
                    name,
                    name_lower,
                    exe_path,
                    cwd,
                    cmdline,
                    cpu_percent: proc_info.cpu_usage(),
                    memory_bytes: proc_info.memory(),
                    start_time_secs: proc_info.start_time(),
                },
            );
        }

        // Identify all tracked terminal/shell PIDs
        let mut tracked_pids: HashSet<u32> = HashSet::new();
        for proc in procs.values() {
            if proc.pid == self.self_pid {
                continue;
            }
            if is_shell_or_terminal(&proc.name_lower) {
                tracked_pids.insert(proc.pid);
            }
        }

        // Also track standalone/orphaned dev runtimes (bun, node, python, etc.) that are
        // listening on a TCP port or have a console window, when they are NOT already inside
        // a tracked shell (e.g. a startup script spawned `bun.exe` and then exited!).
        for proc in procs.values() {
            if proc.pid == self.self_pid || tracked_pids.contains(&proc.pid) {
                continue;
            }
            if !DEV_RUNTIME_EXES.contains(&proc.name_lower.as_str()) {
                continue;
            }
            let has_ports = ports_by_pid.get(&proc.pid).is_some_and(|p| !p.is_empty());
            let has_console_child = children_map.get(&proc.pid).is_some_and(|kids| {
                kids.iter().any(|k| {
                    procs
                        .get(k)
                        .is_some_and(|kp| is_console_helper_name(&kp.name_lower))
                })
            });
            if !has_ports && !has_console_child {
                continue;
            }
            // Only promote to top-level if no living ancestor is already a tracked shell
            let ancestry = build_ancestry(proc.pid, &procs);
            let inside_tracked_shell = ancestry.iter().any(|a| {
                procs
                    .get(&a.pid)
                    .is_some_and(|ap| is_shell_or_terminal(&ap.name_lower))
            });
            if !inside_tracked_shell {
                tracked_pids.insert(proc.pid);
            }
        }

        // Collect all visible/minimized terminal-host windows (e.g. WindowsTerminal.exe can own N windows!)
        let mut terminal_host_windows: Vec<WindowInfo> = Vec::new();
        for &pid in &tracked_pids {
            if let Some(proc) = procs.get(&pid) {
                if TERMINAL_HOST_EXES.contains(&proc.name_lower.as_str()) {
                    if let Some(wins) = windows_by_pid.get(&pid) {
                        for w in wins {
                            if is_real_terminal_window(w) {
                                terminal_host_windows.push(w.clone());
                            }
                        }
                    }
                }
            }
        }

        let mut instances = Vec::new();

        for &pid in &tracked_pids {
            let Some(proc) = procs.get(&pid) else {
                continue;
            };

            let ancestry = build_ancestry(pid, &procs);
            let is_helper = is_console_helper_instance(proc, &procs);
            let is_system_service = detect_system_service(&ancestry, proc);

            // Collect descendant processes (excluding child console helpers for cleanliness in children summary)
            let mut descendants = Vec::new();
            collect_descendants(
                pid,
                1,
                &children_map,
                &procs,
                &ports_by_pid,
                &mut descendants,
                &mut HashSet::new(),
            );

            // Aggregate windows: own windows + immediate child conhost/OpenConsole windows
            let mut instance_windows: Vec<WindowInfo> =
                windows_by_pid.get(&pid).cloned().unwrap_or_default();

            if instance_windows.is_empty() && !is_console_helper_name(&proc.name_lower) {
                if let Some(direct_children) = children_map.get(&pid) {
                    for &child_pid in direct_children {
                        if let Some(child_proc) = procs.get(&child_pid) {
                            if is_console_helper_name(&child_proc.name_lower) {
                                if let Some(cwins) = windows_by_pid.get(&child_pid) {
                                    instance_windows.extend(cwins.iter().cloned());
                                }
                            }
                        }
                    }
                }
            }

            // Aggregate listening ports across self and descendants
            let mut all_ports: Vec<u16> = ports_by_pid.get(&pid).cloned().unwrap_or_default();
            for d in &descendants {
                all_ports.extend_from_slice(&d.listening_ports);
            }
            all_ports.sort_unstable();
            all_ports.dedup();

            // Aggregate tree CPU & memory
            let total_tree_memory_bytes = proc.memory_bytes
                + descendants.iter().map(|d| d.memory_bytes).sum::<u64>();
            let total_tree_cpu_percent = proc.cpu_percent
                + descendants.iter().map(|d| d.cpu_percent).sum::<f32>();

            let (owner_app, owner_category) =
                attribute_owner(proc, &ancestry, &descendants, is_system_service);
            let purpose = classify_purpose(proc, &descendants, &all_ports, is_helper);

            // Multi-window terminal hosts (like WindowsTerminal.exe on Windows 11):
            // emit one TerminalInstance per visible/minimized window so every open terminal window
            // shows up individually instead of collapsing into a single card!
            if TERMINAL_HOST_EXES.contains(&proc.name_lower.as_str()) {
                let real_wins: Vec<WindowInfo> = instance_windows
                    .iter()
                    .filter(|w| is_real_terminal_window(w))
                    .cloned()
                    .collect();

                if real_wins.len() > 1 {
                    let total_wins = real_wins.len();
                    for (idx, win) in real_wins.into_iter().enumerate() {
                        let vis = if win.is_visible && win.is_minimized {
                            VisibilityState::Minimized
                        } else if win.is_visible {
                            VisibilityState::Visible
                        } else {
                            VisibilityState::Hidden
                        };
                        instances.push(TerminalInstance {
                            id: format!("pid:{}:hwnd:0x{:X}", proc.pid, win.hwnd),
                            pid: proc.pid,
                            name: proc.name.clone(),
                            exe_path: proc.exe_path.clone(),
                            cwd: proc.cwd.clone(),
                            cmdline: proc.cmdline.clone(),
                            cpu_percent: proc.cpu_percent,
                            memory_bytes: proc.memory_bytes,
                            total_tree_memory_bytes,
                            total_tree_cpu_percent,
                            start_time_secs: proc.start_time_secs,
                            visibility: vis,
                            primary_window: Some(win),
                            window_index: Some((idx + 1, total_wins)),
                            owner_app: owner_app.clone(),
                            owner_category,
                            purpose,
                            ancestry: ancestry.clone(),
                            children: descendants.clone(),
                            listening_ports: all_ports.clone(),
                            is_system_service: false,
                            is_console_helper: false,
                        });
                    }
                    continue;
                }
            }

            // If a shell has no direct window, check if it's hosted inside a parent terminal
            // OR delegated to a visible WindowsTerminal.exe window via Windows 11 Default Terminal Delegation
            let mut primary_window = pick_primary_window(&instance_windows);
            if primary_window.as_ref().is_none_or(|w| !w.is_visible)
                && !is_console_helper_name(&proc.name_lower)
            {
                if let Some(host_win) = find_host_terminal_window(&ancestry, &windows_by_pid) {
                    primary_window = Some(host_win);
                } else if let Some(delegated_win) =
                    match_delegated_terminal_window(proc, &descendants, &terminal_host_windows)
                {
                    primary_window = Some(delegated_win);
                }
            }

            let visibility = match &primary_window {
                Some(w) if w.is_visible && w.is_minimized => VisibilityState::Minimized,
                Some(w) if w.is_visible => VisibilityState::Visible,
                _ => VisibilityState::Hidden,
            };

            instances.push(TerminalInstance {
                id: format!("pid:{}", proc.pid),
                pid: proc.pid,
                name: proc.name.clone(),
                exe_path: proc.exe_path.clone(),
                cwd: proc.cwd.clone(),
                cmdline: proc.cmdline.clone(),
                cpu_percent: proc.cpu_percent,
                memory_bytes: proc.memory_bytes,
                total_tree_memory_bytes,
                total_tree_cpu_percent,
                start_time_secs: proc.start_time_secs,
                visibility,
                primary_window,
                window_index: None,
                owner_app,
                owner_category,
                purpose,
                ancestry,
                children: descendants,
                listening_ports: all_ports,
                is_system_service,
                is_console_helper: is_helper,
            });
        }

        instances
    }
}

fn is_shell_or_terminal(name_lower: &str) -> bool {
    SHELL_AND_TERMINAL_EXES.contains(&name_lower)
}

fn is_console_helper_name(name_lower: &str) -> bool {
    CONSOLE_HELPER_EXES.contains(&name_lower)
}

fn is_console_helper_instance(proc: &RawProc, procs: &HashMap<u32, RawProc>) -> bool {
    if !is_console_helper_name(&proc.name_lower) {
        return false;
    }
    // OpenConsole.exe -Embedding is always a Windows Terminal helper process
    if proc.name_lower == "openconsole.exe" {
        return true;
    }
    // If parent is already a tracked shell, terminal host, or promoted dev runtime, this conhost is a helper
    if let Some(ppid) = proc.parent_pid {
        if let Some(parent) = procs.get(&ppid) {
            if is_shell_or_terminal(&parent.name_lower)
                || DEV_RUNTIME_EXES.contains(&parent.name_lower.as_str())
            {
                return true;
            }
        }
    }
    false
}

fn is_real_terminal_window(w: &WindowInfo) -> bool {
    if !w.is_visible {
        return false;
    }
    if w.class_name == "CASCADIA_HOSTING_WINDOW_CLASS"
        || w.class_name == "ConsoleWindowClass"
        || w.class_name == "mintty"
        || w.class_name == "VirtualConsoleClass"
        || w.class_name == "org.wezfurlong.wezterm"
    {
        return true;
    }
    !w.title.trim().is_empty() && (w.is_minimized || (w.width > 50 && w.height > 50))
}

/// Try to match a shell process (cmd.exe / powershell.exe) that was handed off to Windows Terminal
/// via Windows 11 Default Terminal Delegation against one of the visible Windows Terminal windows.
fn match_delegated_terminal_window(
    proc: &RawProc,
    descendants: &[ChildProcessInfo],
    terminal_windows: &[WindowInfo],
) -> Option<WindowInfo> {
    if terminal_windows.is_empty() {
        return None;
    }
    let cmd_lower = proc.cmdline.to_lowercase();
    // Explicitly hidden or pipe-redirected non-interactive shells are genuinely hidden
    if cmd_lower.contains("-windowstyle hidden")
        || cmd_lower.contains("-w hidden")
        || cmd_lower.contains("\\\\.\\pipe\\")
        || cmd_lower.contains("-noninteractive")
    {
        return None;
    }

    // Build candidate needles from exe path, script filenames, CWD, and child processes
    let mut needles: Vec<String> = Vec::new();
    if !proc.exe_path.is_empty() {
        needles.push(proc.exe_path.to_lowercase());
    }
    if !proc.cwd.is_empty() {
        needles.push(proc.cwd.to_lowercase());
    }
    // Extract quoted or .ps1/.bat/.cmd/.py/.js tokens from cmdline
    for token in proc.cmdline.split_whitespace() {
        let cleaned = token.trim_matches('"').trim_matches('\'').to_lowercase();
        if cleaned.ends_with(".ps1")
            || cleaned.ends_with(".bat")
            || cleaned.ends_with(".cmd")
            || cleaned.ends_with(".py")
            || cleaned.ends_with(".js")
            || cleaned.ends_with(".ts")
        {
            if let Some(fname) = cleaned.rsplit(['\\', '/']).next() {
                if fname.len() >= 3 {
                    needles.push(fname.to_string());
                }
            }
            needles.push(cleaned);
        }
    }
    for d in descendants {
        let dname = d.name.to_lowercase();
        if !dname.is_empty() {
            needles.push(dname);
        }
    }

    // Score each visible Windows Terminal window by how specifically its title matches this shell
    let mut best: Option<(usize, &WindowInfo)> = None;
    for win in terminal_windows {
        let title_lower = win.title.to_lowercase();
        if title_lower.is_empty() {
            continue;
        }
        let mut score = 0usize;
        for needle in &needles {
            if !needle.is_empty() && title_lower.contains(needle.as_str()) {
                score += needle.len();
            }
        }
        if score > 0 && best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, win));
        }
    }

    best.map(|(_, w)| w.clone())
}

fn build_ancestry(pid: u32, procs: &HashMap<u32, RawProc>) -> Vec<AncestorInfo> {
    let mut chain = Vec::new();
    let mut visited = HashSet::new();
    let mut current = pid;
    visited.insert(current);

    while let Some(proc) = procs.get(&current) {
        let Some(ppid) = proc.parent_pid else {
            break;
        };
        if ppid == 0 || !visited.insert(ppid) {
            break;
        }
        if let Some(parent) = procs.get(&ppid) {
            chain.push(AncestorInfo {
                pid: parent.pid,
                name: parent.name.clone(),
            });
            current = ppid;
        } else {
            chain.push(AncestorInfo {
                pid: ppid,
                name: format!("PID {} (exited)", ppid),
            });
            break;
        }
        if chain.len() >= 12 {
            break;
        }
    }

    chain.reverse();
    chain
}

fn collect_descendants(
    parent_pid: u32,
    depth: usize,
    children_map: &HashMap<u32, Vec<u32>>,
    procs: &HashMap<u32, RawProc>,
    ports_by_pid: &HashMap<u32, Vec<u16>>,
    out: &mut Vec<ChildProcessInfo>,
    visited: &mut HashSet<u32>,
) {
    if depth > 6 {
        return;
    }
    let Some(kids) = children_map.get(&parent_pid) else {
        return;
    };

    for &child_pid in kids {
        if !visited.insert(child_pid) {
            continue;
        }
        if let Some(p) = procs.get(&child_pid) {
            // Skip conhost/OpenConsole noise inside the child list
            if !is_console_helper_name(&p.name_lower) {
                out.push(ChildProcessInfo {
                    pid: p.pid,
                    depth,
                    name: p.name.clone(),
                    cmdline: p.cmdline.clone(),
                    cpu_percent: p.cpu_percent,
                    memory_bytes: p.memory_bytes,
                    listening_ports: ports_by_pid.get(&p.pid).cloned().unwrap_or_default(),
                });
            }
            collect_descendants(
                child_pid,
                depth + 1,
                children_map,
                procs,
                ports_by_pid,
                out,
                visited,
            );
        }
    }
}

fn pick_primary_window(windows: &[WindowInfo]) -> Option<WindowInfo> {
    // Prefer visible non-minimized window with title, then visible, then any
    windows
        .iter()
        .max_by_key(|w| {
            let vis = if w.is_visible { 100 } else { 0 };
            let not_min = if !w.is_minimized { 20 } else { 0 };
            let has_title = if !w.title.trim().is_empty() { 10 } else { 0 };
            let area = if w.width > 0 && w.height > 0 { 5 } else { 0 };
            vis + not_min + has_title + area
        })
        .cloned()
}

fn find_host_terminal_window(
    ancestry: &[AncestorInfo],
    windows_by_pid: &HashMap<u32, Vec<WindowInfo>>,
) -> Option<WindowInfo> {
    for anc in ancestry.iter().rev() {
        let lower = anc.name.to_lowercase();
        if TERMINAL_HOST_EXES.contains(&lower.as_str()) {
            if let Some(wins) = windows_by_pid.get(&anc.pid) {
                if let Some(primary) = pick_primary_window(wins) {
                    if primary.is_visible {
                        return Some(primary);
                    }
                }
            }
        }
    }
    None
}

fn detect_system_service(ancestry: &[AncestorInfo], proc: &RawProc) -> bool {
    // Terminal hosts (WindowsTerminal.exe -Embedding, etc.) are COM-activated via svchost.exe
    // but are ALWAYS user-facing desktop apps, never background system services.
    if TERMINAL_HOST_EXES.contains(&proc.name_lower.as_str()) {
        return false;
    }

    // Only treat conhost.exe spawned directly by core Windows system/service processes as system noise.
    // Actual cmd.exe / powershell.exe instances (even when launched via WmiPrvSE.exe -> svchost.exe)
    // should remain visible to the user by default!
    if proc.name_lower == "conhost.exe" {
        if let Some(direct_parent) = ancestry.last() {
            let l = direct_parent.name.to_lowercase();
            if matches!(
                l.as_str(),
                "services.exe"
                    | "svchost.exe"
                    | "wmiprvse.exe"
                    | "wininit.exe"
                    | "lsass.exe"
                    | "csrss.exe"
                    | "taskhostw.exe"
                    | "searchindexer.exe"
                    | "msmpeng.exe"
                    | "nissrv.exe"
                    | "dllhost.exe"
                    | "sihost.exe"
                    | "spoolsv.exe"
            ) {
                return true;
            }
        }
        return false;
    }

    // For actual shells, only mark as system service if directly spawned by services.exe / wininit.exe / lsass.exe
    if let Some(direct_parent) = ancestry.last() {
        let l = direct_parent.name.to_lowercase();
        if matches!(l.as_str(), "services.exe" | "wininit.exe" | "lsass.exe" | "csrss.exe") {
            return true;
        }
    }

    false
}

fn attribute_owner(
    proc: &RawProc,
    ancestry: &[AncestorInfo],
    descendants: &[ChildProcessInfo],
    is_system_service: bool,
) -> (String, OwnerCategory) {
    // Walk from closest ancestor up to root looking for known apps/tools
    for anc in ancestry.iter().rev() {
        let l = anc.name.to_lowercase();
        if let Some(matched) = match_known_app(&l) {
            return matched;
        }
    }

    // Check cmdline / descendants for AI agents or known tools if parent exited (orphaned shell)
    let combined_cmd = format!(
        "{} {}",
        proc.cmdline.to_lowercase(),
        descendants
            .iter()
            .map(|d| format!("{} {}", d.name.to_lowercase(), d.cmdline.to_lowercase()))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if combined_cmd.contains("t3-code") || combined_cmd.contains("antigravity") {
        return ("T3 Code / Antigravity".to_string(), OwnerCategory::AiTool);
    }
    if combined_cmd.contains("claude") {
        return ("Claude".to_string(), OwnerCategory::AiTool);
    }
    if combined_cmd.contains("codex") {
        return ("Codex".to_string(), OwnerCategory::AiTool);
    }
    if combined_cmd.contains("mcp") {
        return ("MCP Server".to_string(), OwnerCategory::AiTool);
    }

    if is_system_service {
        return ("Windows Service".to_string(), OwnerCategory::System);
    }

    // Fallback: pick the most meaningful non-shell ancestor
    for anc in ancestry.iter().rev() {
        let l = anc.name.to_lowercase();
        if !is_shell_or_terminal(&l)
            && !l.starts_with("pid ")
            && !matches!(l.as_str(), "explorer.exe" | "userinit.exe" | "winlogon.exe")
        {
            let clean = anc.name.strip_suffix(".exe").unwrap_or(&anc.name);
            return (clean.to_string(), OwnerCategory::Other);
        }
    }

    if ancestry
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case("explorer.exe"))
    {
        return ("User (Explorer)".to_string(), OwnerCategory::User);
    }

    if TERMINAL_HOST_EXES.contains(&proc.name_lower.as_str()) {
        let clean = proc.name.strip_suffix(".exe").unwrap_or(&proc.name);
        return (clean.to_string(), OwnerCategory::Terminal);
    }

    ("Orphaned / Direct".to_string(), OwnerCategory::Other)
}

fn match_known_app(name_lower: &str) -> Option<(String, OwnerCategory)> {
    let res = match name_lower {
        "code.exe" | "code - insiders.exe" => ("VS Code", OwnerCategory::Ide),
        "cursor.exe" => ("Cursor", OwnerCategory::Ide),
        "windsurf.exe" => ("Windsurf", OwnerCategory::Ide),
        "zed.exe" => ("Zed", OwnerCategory::Ide),
        "devenv.exe" => ("Visual Studio", OwnerCategory::Ide),
        "idea64.exe" => ("IntelliJ IDEA", OwnerCategory::Ide),
        "rider64.exe" => ("JetBrains Rider", OwnerCategory::Ide),
        "pycharm64.exe" => ("PyCharm", OwnerCategory::Ide),
        "webstorm64.exe" => ("WebStorm", OwnerCategory::Ide),
        "rustrover64.exe" => ("RustRover", OwnerCategory::Ide),
        "goland64.exe" => ("GoLand", OwnerCategory::Ide),
        "clion64.exe" => ("CLion", OwnerCategory::Ide),
        "claude.exe" => ("Claude", OwnerCategory::AiTool),
        "codex.exe" => ("Codex", OwnerCategory::AiTool),
        "antigravity.exe" => ("Antigravity", OwnerCategory::AiTool),
        "t3-code.exe" | "t3code.exe" => ("T3 Code", OwnerCategory::AiTool),
        "ollama.exe" => ("Ollama", OwnerCategory::AiTool),
        "lm studio.exe" => ("LM Studio", OwnerCategory::AiTool),
        "windowsterminal.exe" => ("Windows Terminal", OwnerCategory::Terminal),
        "wezterm-gui.exe" => ("WezTerm", OwnerCategory::Terminal),
        "alacritty.exe" => ("Alacritty", OwnerCategory::Terminal),
        "warp.exe" => ("Warp", OwnerCategory::Terminal),
        "tabby.exe" => ("Tabby", OwnerCategory::Terminal),
        "docker desktop.exe" | "com.docker.backend.exe" => ("Docker", OwnerCategory::DevTool),
        "node.exe" => ("Node.js", OwnerCategory::DevTool),
        "python.exe" | "python3.exe" => ("Python", OwnerCategory::DevTool),
        "cargo.exe" => ("Cargo", OwnerCategory::DevTool),
        "git.exe" => ("Git", OwnerCategory::DevTool),
        _ => return None,
    };
    Some((res.0.to_string(), res.1))
}

fn classify_purpose(
    proc: &RawProc,
    descendants: &[ChildProcessInfo],
    ports: &[u16],
    is_helper: bool,
) -> PurposeTag {
    if is_helper {
        return PurposeTag::SystemHelper;
    }

    let mut blob = format!("{} {}", proc.name_lower, proc.cmdline.to_lowercase());
    for d in descendants {
        blob.push(' ');
        blob.push_str(&d.name.to_lowercase());
        blob.push(' ');
        blob.push_str(&d.cmdline.to_lowercase());
    }

    if blob.contains("claude")
        || blob.contains("codex")
        || blob.contains("mcp")
        || blob.contains("antigravity")
        || blob.contains("copilot")
        || blob.contains("aider")
        || blob.contains("opencode")
        || blob.contains("agent")
    {
        return PurposeTag::AiAgent;
    }

    if !ports.is_empty()
        || blob.contains("vite")
        || blob.contains("next dev")
        || blob.contains("webpack")
        || blob.contains("uvicorn")
        || blob.contains("flask")
        || blob.contains("manage.py runserver")
        || blob.contains("npm run dev")
        || blob.contains("pnpm dev")
        || blob.contains("yarn dev")
        || blob.contains("trunk serve")
    {
        return PurposeTag::DevServer;
    }

    if blob.contains("cargo ")
        || blob.contains("rustc")
        || blob.contains("msbuild")
        || blob.contains("pytest")
        || blob.contains("jest")
        || blob.contains("vitest")
        || blob.contains("tsc ")
        || blob.contains("gradle")
        || blob.contains("mvn ")
        || blob.contains("cmake")
    {
        return PurposeTag::BuildOrTest;
    }

    if blob.contains("git ") || blob.contains("git.exe") || blob.contains("gh ") {
        return PurposeTag::GitOrVcs;
    }

    if TERMINAL_HOST_EXES.contains(&proc.name_lower.as_str()) {
        return PurposeTag::ShellHost;
    }

    if !descendants.is_empty()
        || blob.contains(" -c ")
        || blob.contains(" /c ")
        || blob.contains("-file")
        || blob.contains("-encodedcommand")
    {
        return PurposeTag::Script;
    }

    PurposeTag::IdleShell
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_runs_and_enumerates_system() {
        let mut scanner = ProcessScanner::new();
        let instances = scanner.scan();
        // Verify the scan completes cleanly and produces valid metadata
        for inst in &instances {
            assert!(inst.pid > 0);
            assert!(!inst.name.is_empty());
            assert!(!inst.owner_app.is_empty());
        }
    }
}
