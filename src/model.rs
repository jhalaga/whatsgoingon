use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VisibilityState {
    Visible,
    Minimized,
    Hidden,
}

impl VisibilityState {
    pub fn label(self) -> &'static str {
        match self {
            VisibilityState::Visible => "Visible",
            VisibilityState::Minimized => "Minimized",
            VisibilityState::Hidden => "Hidden",
        }
    }

    pub fn sort_order(self) -> u8 {
        match self {
            VisibilityState::Visible => 0,
            VisibilityState::Minimized => 1,
            VisibilityState::Hidden => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PurposeTag {
    AiAgent,
    DevServer,
    BuildOrTest,
    Script,
    GitOrVcs,
    ShellHost,
    IdleShell,
    SystemHelper,
}

impl PurposeTag {
    pub fn label(self) -> &'static str {
        match self {
            PurposeTag::AiAgent => "AI Agent / MCP",
            PurposeTag::DevServer => "Dev Server",
            PurposeTag::BuildOrTest => "Build / Test",
            PurposeTag::Script => "Script / Tool",
            PurposeTag::GitOrVcs => "Git / VCS",
            PurposeTag::ShellHost => "Terminal Host",
            PurposeTag::IdleShell => "Idle / Blank",
            PurposeTag::SystemHelper => "Console Host",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub owner_pid: u32,
    pub title: String,
    pub class_name: String,
    pub is_visible: bool,
    pub is_minimized: bool,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone)]
pub struct AncestorInfo {
    pub pid: u32,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ChildProcessInfo {
    pub pid: u32,
    pub depth: usize,
    pub name: String,
    pub cmdline: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub listening_ports: Vec<u16>,
}

#[derive(Debug, Clone)]
pub struct TerminalInstance {
    pub id: String,
    pub pid: u32,
    pub name: String,
    pub exe_path: String,
    pub cwd: String,
    pub cmdline: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub total_tree_memory_bytes: u64,
    pub total_tree_cpu_percent: f32,
    pub start_time_secs: u64,
    pub visibility: VisibilityState,
    pub primary_window: Option<WindowInfo>,
    pub window_index: Option<(usize, usize)>,
    pub owner_app: String,
    pub owner_category: OwnerCategory,
    pub purpose: PurposeTag,
    pub ancestry: Vec<AncestorInfo>,
    pub children: Vec<ChildProcessInfo>,
    pub listening_ports: Vec<u16>,
    pub is_system_service: bool,
    pub is_console_helper: bool,
}

impl TerminalInstance {
    pub fn uptime(&self) -> Duration {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Duration::from_secs(now.saturating_sub(self.start_time_secs))
    }

    pub fn display_title(&self) -> String {
        if let Some(ref win) = self.primary_window {
            if !win.title.trim().is_empty() {
                return win.title.clone();
            }
        }
        if let Some(first_child) = self.children.first() {
            return format!("{} → {}", self.name, first_child.name);
        }
        self.name.clone()
    }

    pub fn subtitle_summary(&self) -> String {
        if !self.children.is_empty() {
            let names: Vec<&str> = self
                .children
                .iter()
                .take(3)
                .map(|c| c.name.as_str())
                .collect();
            let extra = if self.children.len() > 3 {
                format!(" +{}", self.children.len() - 3)
            } else {
                String::new()
            };
            format!("Running: {}{}", names.join(", "), extra)
        } else if !self.cmdline.is_empty() && self.cmdline.to_lowercase() != self.name.to_lowercase()
        {
            self.cmdline.clone()
        } else if !self.cwd.is_empty() {
            self.cwd.clone()
        } else {
            "No active child processes".to_string()
        }
    }

    pub fn all_tree_pids(&self) -> Vec<u32> {
        let mut pids = Vec::with_capacity(self.children.len() + 1);
        for c in &self.children {
            pids.push(c.pid);
        }
        pids.push(self.pid);
        pids
    }

    pub fn is_busy(&self) -> bool {
        !self.children.is_empty() || self.total_tree_cpu_percent >= 1.0 || !self.listening_ports.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OwnerCategory {
    Ide,
    AiTool,
    User,
    Terminal,
    DevTool,
    System,
    Other,
}

#[derive(Debug, Clone)]
pub enum ConsolePeekResult {
    Success {
        pid: u32,
        lines: Vec<String>,
        is_blank: bool,
        buffer_cols: i16,
        buffer_rows: i16,
    },
    Unavailable {
        pid: u32,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupBy {
    None,
    OwnerApp,
    Visibility,
}

impl GroupBy {
    pub fn label(self) -> &'static str {
        match self {
            GroupBy::None => "Flat List",
            GroupBy::OwnerApp => "By Owner App",
            GroupBy::Visibility => "By Visibility",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickFilter {
    All,
    Visible,
    Hidden,
    Minimized,
    Active,
    Idle,
    WithPorts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    Visibility,
    Owner,
    Cpu,
    Memory,
    Uptime,
    Pid,
}

impl SortBy {
    pub fn label(self) -> &'static str {
        match self {
            SortBy::Visibility => "Visibility",
            SortBy::Owner => "Owner App",
            SortBy::Cpu => "CPU %",
            SortBy::Memory => "Memory",
            SortBy::Uptime => "Newest",
            SortBy::Pid => "PID",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct DashboardStats {
    pub total: usize,
    pub visible: usize,
    pub minimized: usize,
    pub hidden: usize,
    pub active: usize,
    pub idle: usize,
    pub with_ports: usize,
}

pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("{}d {}h", secs / 86400, (secs % 86400) / 3600)
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}
