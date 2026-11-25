use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Local;
use egui::{Align, Color32, CornerRadius, Layout, Margin, RichText, Stroke, Vec2};

use crate::model::{
    format_bytes, format_duration, ConsolePeekResult, DashboardStats, GroupBy, QuickFilter,
    SortBy, TerminalInstance, VisibilityState,
};
use crate::scanner::ProcessScanner;
use crate::theme::{self, *};
use crate::win32;

struct Toast {
    message: String,
    is_error: bool,
    created_at: Instant,
}

struct PendingKillConfirm {
    pid: u32,
    name: String,
    tree_pids: Vec<u32>,
    is_tree: bool,
}

pub struct WhatsGoingOnApp {
    instances: Vec<TerminalInstance>,
    selected_id: Option<String>,
    search_query: String,
    quick_filter: QuickFilter,
    group_by: GroupBy,
    sort_by: SortBy,
    show_console_helpers: bool,
    show_system_services: bool,
    refresh_interval_secs: u64,
    auto_refresh: bool,
    last_scan_time: String,
    scan_in_progress: bool,

    // Background scan channel
    scan_tx: Sender<()>,
    scan_rx: Receiver<Vec<TerminalInstance>>,
    next_auto_scan: Instant,

    // Background console peek channel
    peek_tx: Sender<u32>,
    peek_rx: Receiver<ConsolePeekResult>,
    peek_result: Option<ConsolePeekResult>,
    peek_loading_pid: Option<u32>,

    // UX state
    toast: Option<Toast>,
    pending_kill: Option<PendingKillConfirm>,
    skip_kill_confirm: bool,
}

impl WhatsGoingOnApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply_theme(&cc.egui_ctx);

        let (scan_req_tx, scan_req_rx) = mpsc::channel::<()>();
        let (scan_res_tx, scan_res_rx) = mpsc::channel::<Vec<TerminalInstance>>();
        let ctx_scan = cc.egui_ctx.clone();

        thread::spawn(move || {
            let mut scanner = ProcessScanner::new();
            // Initial scan right away
            let initial = scanner.scan();
            let _ = scan_res_tx.send(initial);
            ctx_scan.request_repaint();

            while scan_req_rx.recv().is_ok() {
                // Drain any queued duplicate triggers
                while scan_req_rx.try_recv().is_ok() {}
                let results = scanner.scan();
                if scan_res_tx.send(results).is_err() {
                    break;
                }
                ctx_scan.request_repaint();
            }
        });

        let (peek_req_tx, peek_req_rx) = mpsc::channel::<u32>();
        let (peek_res_tx, peek_res_rx) = mpsc::channel::<ConsolePeekResult>();
        let ctx_peek = cc.egui_ctx.clone();

        thread::spawn(move || {
            while let Ok(pid) = peek_req_rx.recv() {
                // Keep only the latest requested PID
                let mut latest_pid = pid;
                while let Ok(newer) = peek_req_rx.try_recv() {
                    latest_pid = newer;
                }
                let res = win32::peek_console_output(latest_pid);
                if peek_res_tx.send(res).is_err() {
                    break;
                }
                ctx_peek.request_repaint();
            }
        });

        Self {
            instances: Vec::new(),
            selected_id: None,
            search_query: String::new(),
            quick_filter: QuickFilter::All,
            group_by: GroupBy::OwnerApp,
            sort_by: SortBy::Visibility,
            show_console_helpers: false,
            show_system_services: false,
            refresh_interval_secs: 2,
            auto_refresh: true,
            last_scan_time: "Scanning...".to_string(),
            scan_in_progress: true,
            scan_tx: scan_req_tx,
            scan_rx: scan_res_rx,
            next_auto_scan: Instant::now() + Duration::from_secs(2),
            peek_tx: peek_req_tx,
            peek_rx: peek_res_rx,
            peek_result: None,
            peek_loading_pid: None,
            toast: None,
            pending_kill: None,
            skip_kill_confirm: false,
        }
    }

    fn trigger_scan(&mut self) {
        if !self.scan_in_progress {
            self.scan_in_progress = true;
            let _ = self.scan_tx.send(());
        }
        self.next_auto_scan = Instant::now() + Duration::from_secs(self.refresh_interval_secs);
    }

    fn trigger_peek(&mut self, pid: u32) {
        self.peek_loading_pid = Some(pid);
        let _ = self.peek_tx.send(pid);
    }

    fn set_toast(&mut self, message: impl Into<String>, is_error: bool) {
        self.toast = Some(Toast {
            message: message.into(),
            is_error,
            created_at: Instant::now(),
        });
    }

    fn poll_background(&mut self) {
        while let Ok(new_instances) = self.scan_rx.try_recv() {
            self.instances = new_instances;
            self.scan_in_progress = false;
            self.last_scan_time = Local::now().format("%H:%M:%S").to_string();

            // Keep selection valid
            if let Some(ref sel) = self.selected_id {
                if !self.instances.iter().any(|i| &i.id == sel) {
                    self.selected_id = None;
                }
            }
        }

        while let Ok(peek) = self.peek_rx.try_recv() {
            self.peek_loading_pid = None;
            self.peek_result = Some(peek);
        }

        if self.auto_refresh && Instant::now() >= self.next_auto_scan {
            self.trigger_scan();
        }

        if let Some(ref toast) = self.toast {
            if toast.created_at.elapsed() > Duration::from_secs(4) {
                self.toast = None;
            }
        }
    }

    fn base_visible_instances(&self) -> Vec<&TerminalInstance> {
        self.instances
            .iter()
            .filter(|i| {
                if !self.show_console_helpers && i.is_console_helper {
                    return false;
                }
                if !self.show_system_services && i.is_system_service {
                    return false;
                }
                true
            })
            .collect()
    }

    fn compute_stats(&self, base: &[&TerminalInstance]) -> DashboardStats {
        let mut s = DashboardStats::default();
        s.total = base.len();
        for inst in base {
            match inst.visibility {
                VisibilityState::Visible => s.visible += 1,
                VisibilityState::Minimized => s.minimized += 1,
                VisibilityState::Hidden => s.hidden += 1,
            }
            if inst.is_busy() {
                s.active += 1;
            } else {
                s.idle += 1;
            }
            if !inst.listening_ports.is_empty() {
                s.with_ports += 1;
            }
        }
        s
    }

    fn filtered_and_sorted<'a>(&self, base: &[&'a TerminalInstance]) -> Vec<&'a TerminalInstance> {
        let q = self.search_query.trim().to_lowercase();
        let mut list: Vec<&TerminalInstance> = base
            .iter()
            .copied()
            .filter(|inst| match self.quick_filter {
                QuickFilter::All => true,
                QuickFilter::Visible => inst.visibility == VisibilityState::Visible,
                QuickFilter::Hidden => inst.visibility == VisibilityState::Hidden,
                QuickFilter::Minimized => inst.visibility == VisibilityState::Minimized,
                QuickFilter::Active => inst.is_busy(),
                QuickFilter::Idle => !inst.is_busy(),
                QuickFilter::WithPorts => !inst.listening_ports.is_empty(),
            })
            .filter(|inst| {
                if q.is_empty() {
                    return true;
                }
                inst.pid.to_string().contains(&q)
                    || inst.name.to_lowercase().contains(&q)
                    || inst.owner_app.to_lowercase().contains(&q)
                    || inst.cmdline.to_lowercase().contains(&q)
                    || inst.cwd.to_lowercase().contains(&q)
                    || inst.purpose.label().to_lowercase().contains(&q)
                    || inst
                        .primary_window
                        .as_ref()
                        .is_some_and(|w| w.title.to_lowercase().contains(&q))
                    || inst
                        .children
                        .iter()
                        .any(|c| c.name.to_lowercase().contains(&q) || c.cmdline.to_lowercase().contains(&q))
                    || inst
                        .listening_ports
                        .iter()
                        .any(|p| p.to_string().contains(&q))
            })
            .collect();

        match self.sort_by {
            SortBy::Visibility => list.sort_by(|a, b| {
                a.visibility
                    .sort_order()
                    .cmp(&b.visibility.sort_order())
                    .then_with(|| b.is_busy().cmp(&a.is_busy()))
                    .then_with(|| a.owner_app.cmp(&b.owner_app))
                    .then_with(|| a.pid.cmp(&b.pid))
            }),
            SortBy::Owner => list.sort_by(|a, b| {
                a.owner_app
                    .cmp(&b.owner_app)
                    .then_with(|| a.visibility.sort_order().cmp(&b.visibility.sort_order()))
                    .then_with(|| a.pid.cmp(&b.pid))
            }),
            SortBy::Cpu => list.sort_by(|a, b| {
                b.total_tree_cpu_percent
                    .partial_cmp(&a.total_tree_cpu_percent)
                    .unwrap_or(std::cmp::Ordering::Equal)
            }),
            SortBy::Memory => list.sort_by(|a, b| b.total_tree_memory_bytes.cmp(&a.total_tree_memory_bytes)),
            SortBy::Uptime => list.sort_by(|a, b| b.start_time_secs.cmp(&a.start_time_secs)),
            SortBy::Pid => list.sort_by(|a, b| a.pid.cmp(&b.pid)),
        }

        list
    }

    fn execute_kill(&mut self, pid: u32, tree_pids: &[u32], is_tree: bool) {
        if is_tree {
            let (killed, errors) = win32::terminate_process_tree(tree_pids);
            if errors.is_empty() {
                self.set_toast(
                    format!("Terminated process tree ({} process(es))", killed),
                    false,
                );
            } else {
                self.set_toast(
                    format!(
                        "Terminated {} process(es); {} error(s): {}",
                        killed,
                        errors.len(),
                        errors[0]
                    ),
                    true,
                );
            }
        } else {
            match win32::terminate_process(pid) {
                Ok(()) => self.set_toast(format!("Terminated PID {}", pid), false),
                Err(e) => self.set_toast(e, true),
            }
        }
        self.trigger_scan();
    }

    fn request_kill(&mut self, inst: &TerminalInstance, is_tree: bool) {
        let tree_pids = inst.all_tree_pids();
        if self.skip_kill_confirm {
            self.execute_kill(inst.pid, &tree_pids, is_tree);
        } else {
            self.pending_kill = Some(PendingKillConfirm {
                pid: inst.pid,
                name: inst.name.clone(),
                tree_pids,
                is_tree,
            });
        }
    }
}

impl eframe::App for WhatsGoingOnApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background();

        // Keep repainting smoothly for auto-refresh countdown & toast fade
        if self.auto_refresh {
            ctx.request_repaint_after(Duration::from_millis(500));
        }

        // Snapshot cloned data for UI rendering so actions can mutate `self` cleanly
        let base_count_stats;
        let filtered_ids: Vec<String>;
        {
            let base = self.base_visible_instances();
            base_count_stats = self.compute_stats(&base);
            filtered_ids = self
                .filtered_and_sorted(&base)
                .into_iter()
                .map(|i| i.id.clone())
                .collect();
        }

        // Top Bar
        egui::TopBottomPanel::top("top_header")
            .frame(
                egui::Frame::new()
                    .fill(BG_SURFACE)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ctx, |ui| {
                self.render_header(ui, &base_count_stats);
            });

        // Bottom Status / Toast Bar
        egui::TopBottomPanel::bottom("bottom_status")
            .frame(
                egui::Frame::new()
                    .fill(BG_SURFACE)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .inner_margin(Margin::symmetric(16, 6)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(ref toast) = self.toast {
                        let color = if toast.is_error {
                            ACCENT_RED
                        } else {
                            ACCENT_GREEN
                        };
                        ui.label(RichText::new(&toast.message).color(color).strong());
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "Showing {} of {} terminal/shell instances",
                                filtered_ids.len(),
                                base_count_stats.total
                            ))
                            .color(TEXT_SECONDARY)
                            .size(12.0),
                        );
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let status = if self.scan_in_progress {
                            "Scanning...".to_string()
                        } else {
                            format!("Last updated {}", self.last_scan_time)
                        };
                        ui.label(RichText::new(status).color(TEXT_MUTED).size(11.5));
                    });
                });
            });

        // Right Inspector Panel
        let selected_instance = self
            .selected_id
            .as_ref()
            .and_then(|id| self.instances.iter().find(|i| &i.id == id))
            .cloned();

        egui::SidePanel::right("inspector_panel")
            .resizable(true)
            .default_width(470.0)
            .min_width(360.0)
            .max_width(720.0)
            .frame(
                egui::Frame::new()
                    .fill(BG_SURFACE)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .inner_margin(Margin::same(14)),
            )
            .show(ctx, |ui| {
                if let Some(inst) = selected_instance {
                    self.render_inspector(ui, &inst);
                } else {
                    self.render_empty_inspector(ui);
                }
            });

        // Center List Panel
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG_BASE).inner_margin(Margin::same(14)))
            .show(ctx, |ui| {
                self.render_toolbar(ui);
                ui.add_space(10.0);
                self.render_instance_list(ui, &filtered_ids);
            });

        // Kill confirmation modal
        self.render_kill_modal(ctx);
    }
}

impl WhatsGoingOnApp {
    fn render_header(&mut self, ui: &mut egui::Ui, stats: &DashboardStats) {
        ui.horizontal(|ui| {
            // Title & tagline
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("◉ WhatsGoingOn (wgo)")
                            .size(19.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );
                    ui.add_space(6.0);
                    badge(ui, "TERMINAL MONITOR", ACCENT_BLUE);
                });
                ui.label(
                    RichText::new("See every visible, minimized, and hidden terminal/cmd instance & what owns it")
                        .size(11.5)
                        .color(TEXT_MUTED),
                );
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button(RichText::new("↻ Refresh").strong())
                    .on_hover_text("Scan all running processes & windows now")
                    .clicked()
                {
                    self.trigger_scan();
                }

                egui::ComboBox::from_id_salt("refresh_rate")
                    .width(92.0)
                    .selected_text(if self.auto_refresh {
                        format!("Auto {}s", self.refresh_interval_secs)
                    } else {
                        "Paused".to_string()
                    })
                    .show_ui(ui, |ui| {
                        for &(secs, label) in &[(1, "Every 1s"), (2, "Every 2s"), (5, "Every 5s")] {
                            if ui
                                .selectable_label(
                                    self.auto_refresh && self.refresh_interval_secs == secs,
                                    label,
                                )
                                .clicked()
                            {
                                self.auto_refresh = true;
                                self.refresh_interval_secs = secs;
                                self.next_auto_scan = Instant::now() + Duration::from_secs(secs);
                            }
                        }
                        if ui.selectable_label(!self.auto_refresh, "Paused").clicked() {
                            self.auto_refresh = false;
                        }
                    });
            });
        });

        ui.add_space(10.0);

        // Clickable KPI Filter Pills
        ui.horizontal_wrapped(|ui| {
            self.kpi_pill(ui, "All", stats.total, TEXT_PRIMARY, QuickFilter::All);
            self.kpi_pill(ui, "● Visible", stats.visible, ACCENT_GREEN, QuickFilter::Visible);
            self.kpi_pill(ui, "● Hidden", stats.hidden, ACCENT_PURPLE, QuickFilter::Hidden);
            self.kpi_pill(ui, "● Minimized", stats.minimized, ACCENT_AMBER, QuickFilter::Minimized);
            self.kpi_pill(ui, "⚡ Active", stats.active, ACCENT_CYAN, QuickFilter::Active);
            self.kpi_pill(ui, "○ Idle / Blank", stats.idle, TEXT_SECONDARY, QuickFilter::Idle);
            self.kpi_pill(ui, "⇄ Listening Ports", stats.with_ports, ACCENT_BLUE, QuickFilter::WithPorts);
        });
    }

    fn kpi_pill(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        count: usize,
        color: Color32,
        filter: QuickFilter,
    ) {
        let selected = self.quick_filter == filter;
        let fill = if selected {
            Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 42)
        } else {
            BG_CARD
        };
        let stroke = if selected {
            Stroke::new(1.5, color)
        } else {
            Stroke::new(1.0, BORDER_SUBTLE)
        };

        let resp = egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).size(12.0).color(color));
                    ui.label(
                        RichText::new(count.to_string())
                            .size(13.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );
                });
            })
            .response
            .interact(egui::Sense::click());

        if resp.clicked() {
            self.quick_filter = if selected && filter != QuickFilter::All {
                QuickFilter::All
            } else {
                filter
            };
        }
    }

    fn render_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Search app, PID, command, folder, port, child...")
                    .desired_width(280.0),
            );
            if !self.search_query.is_empty() && ui.small_button("✕").clicked() {
                self.search_query.clear();
            }

            ui.separator();

            ui.label(RichText::new("Group:").color(TEXT_MUTED).size(12.0));
            egui::ComboBox::from_id_salt("group_by")
                .width(120.0)
                .selected_text(self.group_by.label())
                .show_ui(ui, |ui| {
                    for g in [GroupBy::OwnerApp, GroupBy::Visibility, GroupBy::None] {
                        ui.selectable_value(&mut self.group_by, g, g.label());
                    }
                });

            ui.label(RichText::new("Sort:").color(TEXT_MUTED).size(12.0));
            egui::ComboBox::from_id_salt("sort_by")
                .width(100.0)
                .selected_text(self.sort_by.label())
                .show_ui(ui, |ui| {
                    for s in [
                        SortBy::Visibility,
                        SortBy::Owner,
                        SortBy::Cpu,
                        SortBy::Memory,
                        SortBy::Uptime,
                        SortBy::Pid,
                    ] {
                        ui.selectable_value(&mut self.sort_by, s, s.label());
                    }
                });

            ui.separator();
            ui.checkbox(&mut self.show_console_helpers, "Console hosts")
                .on_hover_text("Include helper conhost.exe / OpenConsole.exe processes");
            ui.checkbox(&mut self.show_system_services, "System")
                .on_hover_text("Include Windows service console hosts");
        });
    }

    fn render_instance_list(&mut self, ui: &mut egui::Ui, filtered_ids: &[String]) {
        if filtered_ids.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("No terminal or shell instances match your current filter.")
                        .color(TEXT_MUTED)
                        .size(14.0),
                );
            });
            return;
        }

        // Clone matching instances to avoid borrow conflicts with action buttons
        let items: Vec<TerminalInstance> = filtered_ids
            .iter()
            .filter_map(|id| self.instances.iter().find(|i| &i.id == id).cloned())
            .collect();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| match self.group_by {
                GroupBy::None => {
                    for inst in &items {
                        self.render_instance_card(ui, inst);
                        ui.add_space(6.0);
                    }
                }
                GroupBy::OwnerApp => {
                    let mut groups: BTreeMap<String, Vec<&TerminalInstance>> = BTreeMap::new();
                    for inst in &items {
                        groups.entry(inst.owner_app.clone()).or_default().push(inst);
                    }
                    for (owner, group_items) in groups {
                        let header = format!("{}  ({})", owner, group_items.len());
                        egui::CollapsingHeader::new(
                            RichText::new(header).strong().size(13.5).color(TEXT_PRIMARY),
                        )
                        .default_open(true)
                        .show(ui, |ui| {
                            for inst in group_items {
                                self.render_instance_card(ui, inst);
                                ui.add_space(5.0);
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                GroupBy::Visibility => {
                    for vis in [
                        VisibilityState::Visible,
                        VisibilityState::Minimized,
                        VisibilityState::Hidden,
                    ] {
                        let group_items: Vec<&TerminalInstance> =
                            items.iter().filter(|i| i.visibility == vis).collect();
                        if group_items.is_empty() {
                            continue;
                        }
                        let header = format!("{}  ({})", vis.label(), group_items.len());
                        egui::CollapsingHeader::new(
                            RichText::new(header)
                                .strong()
                                .size(13.5)
                                .color(visibility_color(vis)),
                        )
                        .default_open(true)
                        .show(ui, |ui| {
                            for inst in group_items {
                                self.render_instance_card(ui, inst);
                                ui.add_space(5.0);
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
            });
    }

    fn render_instance_card(&mut self, ui: &mut egui::Ui, inst: &TerminalInstance) {
        let is_selected = self.selected_id.as_deref() == Some(inst.id.as_str());
        let fill = if is_selected { BG_CARD_SELECTED } else { BG_CARD };
        let stroke = if is_selected {
            Stroke::new(1.5, BORDER_ACCENT)
        } else {
            Stroke::new(1.0, BORDER_SUBTLE)
        };

        let frame_resp = egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());

                // Row 1: Visibility dot, Title/Name, PID, Quick Actions
                ui.horizontal(|ui| {
                    let vis_col = visibility_color(inst.visibility);
                    ui.label(RichText::new("●").color(vis_col).size(13.0));

                    let title = inst.display_title();
                    let truncated_title = if title.chars().count() > 54 {
                        format!("{}…", title.chars().take(53).collect::<String>())
                    } else {
                        title
                    };
                    ui.label(RichText::new(truncated_title).strong().color(TEXT_PRIMARY));
                    ui.label(
                        RichText::new(format!("PID {}", inst.pid))
                            .monospace()
                            .size(11.5)
                            .color(TEXT_MUTED),
                    );

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let (Some(_), Some(win)) = (inst.window_index, &inst.primary_window) {
                            if ui
                                .small_button(RichText::new("✕").color(ACCENT_RED))
                                .on_hover_text("Close this window (keeps other Windows Terminal windows open)")
                                .clicked()
                            {
                                match win32::close_window(win.hwnd) {
                                    Ok(()) => {
                                        self.set_toast("Sent close signal to window", false);
                                        self.trigger_scan();
                                    }
                                    Err(e) => self.set_toast(e, true),
                                }
                            }
                        } else if ui
                            .small_button(RichText::new("✕").color(ACCENT_RED))
                            .on_hover_text("Kill process tree")
                            .clicked()
                        {
                            self.request_kill(inst, true);
                        }

                        if let Some(ref win) = inst.primary_window {
                            if ui
                                .small_button("↗ Focus")
                                .on_hover_text("Bring window to front")
                                .clicked()
                            {
                                match win32::focus_window(win.hwnd) {
                                    Ok(()) => self.set_toast(
                                        format!("Focused {} (PID {})", inst.name, inst.pid),
                                        false,
                                    ),
                                    Err(e) => self.set_toast(e, true),
                                }
                            }
                        }

                        ui.label(
                            RichText::new(format!(
                                "{} · {:.1}% · {}",
                                format_bytes(inst.total_tree_memory_bytes),
                                inst.total_tree_cpu_percent,
                                format_duration(inst.uptime())
                            ))
                            .size(11.0)
                            .color(TEXT_MUTED),
                        );
                    });
                });

                ui.add_space(4.0);

                // Row 2: Owner badge, Shell name, Purpose tag, Ports
                ui.horizontal_wrapped(|ui| {
                    badge(ui, &inst.owner_app, owner_color(inst.owner_category));
                    badge(ui, inst.visibility.label(), visibility_color(inst.visibility));
                    if let Some((idx, total)) = inst.window_index {
                        badge(ui, &format!("Window {}/{}", idx, total), ACCENT_BLUE);
                    }
                    badge(ui, inst.purpose.label(), purpose_color(inst.purpose));

                    if !inst.name.is_empty() {
                        ui.label(
                            RichText::new(&inst.name)
                                .monospace()
                                .size(11.5)
                                .color(TEXT_SECONDARY),
                        );
                    }

                    for port in inst.listening_ports.iter().take(4) {
                        badge(ui, &format!(":{}", port), ACCENT_CYAN);
                    }
                    if inst.listening_ports.len() > 4 {
                        badge(
                            ui,
                            &format!("+{} ports", inst.listening_ports.len() - 4),
                            ACCENT_CYAN,
                        );
                    }
                });

                ui.add_space(3.0);

                // Row 3: Subtitle (running children / command / cwd)
                let sub = inst.subtitle_summary();
                let truncated_sub = if sub.chars().count() > 105 {
                    format!("{}…", sub.chars().take(104).collect::<String>())
                } else {
                    sub
                };
                ui.label(RichText::new(truncated_sub).size(11.5).color(TEXT_SECONDARY));
            })
            .response;

        if frame_resp.interact(egui::Sense::click()).clicked() {
            if self.selected_id.as_deref() != Some(inst.id.as_str()) {
                self.selected_id = Some(inst.id.clone());
                self.peek_result = None;
                self.trigger_peek(inst.pid);
            }
        }
    }

    fn render_empty_inspector(&self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Select a terminal instance").size(16.0).strong().color(TEXT_PRIMARY));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Click any card on the left to inspect:\n\
                         • Which app or agent spawned it (ancestry chain)\n\
                         • What child processes & ports are running inside\n\
                         • Live console output peek (even for hidden/blank windows)\n\
                         • Quick actions: Focus, Show/Hide, Open Folder, Kill Tree",
                    )
                    .color(TEXT_SECONDARY)
                    .size(12.5),
                );
            });
        });
    }

    fn render_inspector(&mut self, ui: &mut egui::Ui, inst: &TerminalInstance) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Header
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(&inst.name)
                            .size(18.0)
                            .strong()
                            .color(TEXT_PRIMARY),
                    );
                    ui.label(
                        RichText::new(format!("PID {}", inst.pid))
                            .monospace()
                            .size(13.0)
                            .color(TEXT_MUTED),
                    );
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    badge(ui, &inst.owner_app, owner_color(inst.owner_category));
                    badge(ui, inst.visibility.label(), visibility_color(inst.visibility));
                    if let Some((idx, total)) = inst.window_index {
                        badge(ui, &format!("Window {}/{}", idx, total), ACCENT_BLUE);
                    }
                    badge(ui, inst.purpose.label(), purpose_color(inst.purpose));
                    for port in &inst.listening_ports {
                        badge(ui, &format!("PORT :{}", port), ACCENT_CYAN);
                    }
                });

                ui.add_space(10.0);

                // Action Bar
                ui.horizontal_wrapped(|ui| {
                    if let Some(ref win) = inst.primary_window {
                        if ui.button("↗ Focus Window").clicked() {
                            match win32::focus_window(win.hwnd) {
                                Ok(()) => self.set_toast("Brought window to foreground", false),
                                Err(e) => self.set_toast(e, true),
                            }
                        }
                        if win.is_visible {
                            if ui.button("— Minimize").clicked() {
                                match win32::minimize_window(win.hwnd) {
                                    Ok(()) => {
                                        self.set_toast("Minimized window", false);
                                        self.trigger_scan();
                                    }
                                    Err(e) => self.set_toast(e, true),
                                }
                            }
                            if ui.button("👁 Hide").clicked() {
                                match win32::set_window_visible(win.hwnd, false) {
                                    Ok(()) => {
                                        self.set_toast("Hidden window", false);
                                        self.trigger_scan();
                                    }
                                    Err(e) => self.set_toast(e, true),
                                }
                            }
                        } else if ui.button("👁 Show Window").clicked() {
                            match win32::set_window_visible(win.hwnd, true) {
                                Ok(()) => {
                                    self.set_toast("Shown window", false);
                                    self.trigger_scan();
                                }
                                Err(e) => self.set_toast(e, true),
                            }
                        }
                    }

                    if !inst.cwd.is_empty() && ui.button("📁 Open Folder").clicked() {
                        match win32::open_in_explorer(&inst.cwd, false) {
                            Ok(()) => self.set_toast("Opened working directory", false),
                            Err(e) => self.set_toast(e, true),
                        }
                    }

                    if !inst.cmdline.is_empty() && ui.button("📋 Copy Cmd").clicked() {
                        ui.ctx().copy_text(inst.cmdline.clone());
                        self.set_toast("Copied command line to clipboard", false);
                    }

                    if ui.button("📄 Copy Summary").clicked() {
                        let summary = format_instance_summary(inst);
                        ui.ctx().copy_text(summary);
                        self.set_toast("Copied diagnostic summary to clipboard", false);
                    }
                });

                ui.add_space(4.0);

                // Danger actions row
                ui.horizontal_wrapped(|ui| {
                    if let (Some((idx, total)), Some(win)) =
                        (inst.window_index, &inst.primary_window)
                    {
                        if ui
                            .button(
                                RichText::new(format!("✕ Close This Window ({}/{})", idx, total))
                                    .color(ACCENT_AMBER)
                                    .strong(),
                            )
                            .on_hover_text("Closes only this window without killing the other Windows Terminal windows")
                            .clicked()
                        {
                            match win32::close_window(win.hwnd) {
                                Ok(()) => {
                                    self.set_toast("Sent close signal to window", false);
                                    self.trigger_scan();
                                }
                                Err(e) => self.set_toast(e, true),
                            }
                        }
                    }

                    let kill_tree_btn = egui::Button::new(
                        RichText::new(format!(
                            "✕ Kill Tree ({} proc{})",
                            inst.children.len() + 1,
                            if inst.children.is_empty() { "" } else { "s" }
                        ))
                        .color(ACCENT_RED)
                        .strong(),
                    );
                    if ui.add(kill_tree_btn).clicked() {
                        self.request_kill(inst, true);
                    }

                    if !inst.children.is_empty()
                        && ui
                            .button(RichText::new("Kill Only This PID").color(ACCENT_AMBER))
                            .clicked()
                    {
                        self.request_kill(inst, false);
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                // Section 1: Ownership / Ancestry Chain
                section_title(ui, "WHO STARTED THIS? (ANCESTRY CHAIN)");
                egui::Frame::new()
                    .fill(BG_CARD)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal_wrapped(|ui| {
                            for anc in &inst.ancestry {
                                ui.label(
                                    RichText::new(format!("{} ({})", anc.name, anc.pid))
                                        .size(12.0)
                                        .color(TEXT_SECONDARY),
                                );
                                ui.label(RichText::new("→").color(TEXT_MUTED));
                            }
                            ui.label(
                                RichText::new(format!("{} ({})", inst.name, inst.pid))
                                    .size(12.0)
                                    .strong()
                                    .color(ACCENT_BLUE),
                            );
                        });
                    });

                ui.add_space(12.0);

                // Section 2: Running Child Processes
                section_title(
                    ui,
                    &format!("WHAT'S RUNNING INSIDE ({})", inst.children.len()),
                );
                egui::Frame::new()
                    .fill(BG_CARD)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if inst.children.is_empty() {
                            ui.label(
                                RichText::new(
                                    "No child processes running inside this shell (idle or finished).",
                                )
                                .color(TEXT_MUTED)
                                .size(12.0),
                            );
                        } else {
                            for child in &inst.children {
                                let indent = "  ".repeat(child.depth.saturating_sub(1));
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("{}↳ {}", indent, child.name))
                                            .strong()
                                            .color(TEXT_PRIMARY),
                                    );
                                    ui.label(
                                        RichText::new(format!("PID {}", child.pid))
                                            .monospace()
                                            .size(11.0)
                                            .color(TEXT_MUTED),
                                    );
                                    for p in &child.listening_ports {
                                        badge(ui, &format!(":{}", p), ACCENT_CYAN);
                                    }
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui
                                            .small_button(RichText::new("✕").color(ACCENT_RED))
                                            .on_hover_text("Kill this child process")
                                            .clicked()
                                        {
                                            match win32::terminate_process(child.pid) {
                                                Ok(()) => {
                                                    self.set_toast(
                                                        format!(
                                                            "Killed child {} (PID {})",
                                                            child.name, child.pid
                                                        ),
                                                        false,
                                                    );
                                                    self.trigger_scan();
                                                }
                                                Err(e) => self.set_toast(e, true),
                                            }
                                        }
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {:.1}%",
                                                format_bytes(child.memory_bytes),
                                                child.cpu_percent
                                            ))
                                            .size(11.0)
                                            .color(TEXT_MUTED),
                                        );
                                    });
                                });
                                if !child.cmdline.is_empty() {
                                    ui.label(
                                        RichText::new(format!("{}   {}", indent, child.cmdline))
                                            .monospace()
                                            .size(11.0)
                                            .color(TEXT_SECONDARY),
                                    );
                                }
                                ui.add_space(4.0);
                            }
                        }
                    });

                ui.add_space(12.0);

                // Section 3: Live Console Output Peek
                ui.horizontal(|ui| {
                    section_title(ui, "CONSOLE OUTPUT PEEK");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let loading = self.peek_loading_pid == Some(inst.pid);
                        if ui
                            .add_enabled(!loading, egui::Button::new("↻ Peek Live Buffer"))
                            .on_hover_text(
                                "Reads the live Win32 console screen buffer (works even for hidden windows)",
                            )
                            .clicked()
                        {
                            self.trigger_peek(inst.pid);
                        }
                    });
                });

                egui::Frame::new()
                    .fill(BG_CONSOLE)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if self.peek_loading_pid == Some(inst.pid) {
                            ui.label(
                                RichText::new("Reading console screen buffer...")
                                    .monospace()
                                    .color(TEXT_MUTED),
                            );
                        } else {
                            match &self.peek_result {
                                Some(ConsolePeekResult::Success {
                                    pid,
                                    lines,
                                    is_blank,
                                    buffer_cols,
                                    buffer_rows,
                                }) if *pid == inst.pid => {
                                    if *is_blank {
                                        ui.label(
                                            RichText::new(format!(
                                                "(Console buffer {}×{} is completely empty / blank black screen)",
                                                buffer_cols, buffer_rows
                                            ))
                                            .monospace()
                                            .color(ACCENT_AMBER),
                                        );
                                    } else {
                                        egui::ScrollArea::vertical()
                                            .id_salt("console_peek_scroll")
                                            .max_height(190.0)
                                            .show(ui, |ui| {
                                                let text = lines.join("\n");
                                                ui.add(
                                                    egui::Label::new(
                                                        RichText::new(text)
                                                            .monospace()
                                                            .size(11.5)
                                                            .color(Color32::from_rgb(205, 224, 215)),
                                                    )
                                                    .selectable(true),
                                                );
                                            });
                                    }
                                }
                                Some(ConsolePeekResult::Unavailable { pid, reason })
                                    if *pid == inst.pid =>
                                {
                                    ui.label(
                                        RichText::new(reason)
                                            .monospace()
                                            .size(11.5)
                                            .color(TEXT_MUTED),
                                    );
                                }
                                _ => {
                                    ui.label(
                                        RichText::new("Click 'Peek Live Buffer' to inspect console output.")
                                            .monospace()
                                            .size(11.5)
                                            .color(TEXT_MUTED),
                                    );
                                }
                            }
                        }
                    });

                ui.add_space(12.0);

                // Section 4: Details & Metadata
                section_title(ui, "PROCESS & WINDOW DETAILS");
                egui::Frame::new()
                    .fill(BG_CARD)
                    .stroke(Stroke::new(1.0, BORDER_SUBTLE))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        detail_row(ui, "Uptime", &format_duration(inst.uptime()));
                        detail_row(
                            ui,
                            "Memory (Tree)",
                            &format!(
                                "{} (self: {})",
                                format_bytes(inst.total_tree_memory_bytes),
                                format_bytes(inst.memory_bytes)
                            ),
                        );
                        detail_row(
                            ui,
                            "CPU (Tree)",
                            &format!(
                                "{:.1}% (self: {:.1}%)",
                                inst.total_tree_cpu_percent, inst.cpu_percent
                            ),
                        );
                        if let Some(ref w) = inst.primary_window {
                            detail_row(
                                ui,
                                "Window Title",
                                if w.title.is_empty() {
                                    "(empty title)"
                                } else {
                                    &w.title
                                },
                            );
                            detail_row(
                                ui,
                                "Window",
                                &format!(
                                    "HWND 0x{:X} · {} · {}×{}{}",
                                    w.hwnd,
                                    w.class_name,
                                    w.width,
                                    w.height,
                                    if w.owner_pid != inst.pid {
                                        format!(" (hosted by PID {})", w.owner_pid)
                                    } else {
                                        String::new()
                                    }
                                ),
                            );
                        } else {
                            detail_row(ui, "Window", "No top-level window (headless / ConPTY)");
                        }

                        if !inst.cwd.is_empty() {
                            detail_row(ui, "Working Dir", &inst.cwd);
                        }
                        if !inst.exe_path.is_empty() {
                            detail_row(ui, "Executable", &inst.exe_path);
                            if ui.small_button("Locate .exe in Explorer").clicked() {
                                match win32::open_in_explorer(&inst.exe_path, true) {
                                    Ok(()) => self.set_toast("Opened executable location", false),
                                    Err(e) => self.set_toast(e, true),
                                }
                            }
                        }
                        if !inst.cmdline.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("Command Line").size(11.0).color(TEXT_MUTED));
                            egui::Frame::new()
                                .fill(BG_CONSOLE)
                                .corner_radius(CornerRadius::same(4))
                                .inner_margin(Margin::same(7))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&inst.cmdline)
                                                .monospace()
                                                .size(11.0)
                                                .color(TEXT_PRIMARY),
                                        )
                                        .wrap()
                                        .selectable(true),
                                    );
                                });
                        }
                    });
            });
    }

    fn render_kill_modal(&mut self, ctx: &egui::Context) {
        let Some(confirm) = self.pending_kill.as_ref() else {
            return;
        };
        let pid = confirm.pid;
        let name = confirm.name.clone();
        let tree_pids = confirm.tree_pids.clone();
        let is_tree = confirm.is_tree;

        let mut close_modal = false;
        let mut do_kill = false;

        egui::Window::new("Confirm Termination")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_min_width(340.0);
                if is_tree {
                    ui.label(format!(
                        "Terminate {} (PID {}) and all {} child process(es)?",
                        name,
                        pid,
                        tree_pids.len().saturating_sub(1)
                    ));
                } else {
                    ui.label(format!("Terminate {} (PID {})?", name, pid));
                }
                ui.add_space(6.0);
                ui.checkbox(
                    &mut self.skip_kill_confirm,
                    "Don't ask again during this session",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .button(RichText::new("Terminate").color(ACCENT_RED).strong())
                        .clicked()
                    {
                        do_kill = true;
                        close_modal = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close_modal = true;
                    }
                });
            });

        if close_modal {
            self.pending_kill = None;
        }
        if do_kill {
            self.execute_kill(pid, &tree_pids, is_tree);
        }
    }
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(11.0)
            .strong()
            .color(TEXT_MUTED),
    );
    ui.add_space(4.0);
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{}:", label)).size(11.5).color(TEXT_MUTED));
        ui.add(
            egui::Label::new(RichText::new(value).size(12.0).color(TEXT_PRIMARY))
                .wrap()
                .selectable(true),
        );
    });
    ui.add_space(2.0);
}

fn format_instance_summary(inst: &TerminalInstance) -> String {
    let ancestry = inst
        .ancestry
        .iter()
        .map(|a| format!("{} ({})", a.name, a.pid))
        .chain(std::iter::once(format!("{} ({})", inst.name, inst.pid)))
        .collect::<Vec<_>>()
        .join(" -> ");

    let children = if inst.children.is_empty() {
        "None".to_string()
    } else {
        inst.children
            .iter()
            .map(|c| format!("{} (PID {}): {}", c.name, c.pid, c.cmdline))
            .collect::<Vec<_>>()
            .join("\n  ")
    };

    let ports = if inst.listening_ports.is_empty() {
        "None".to_string()
    } else {
        inst.listening_ports
            .iter()
            .map(|p| format!(":{}", p))
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        "Process: {} (PID {})\nOwner: {}\nVisibility: {}\nPurpose: {}\nAncestry: {}\nCWD: {}\nCommand: {}\nListening Ports: {}\nChildren:\n  {}",
        inst.name,
        inst.pid,
        inst.owner_app,
        inst.visibility.label(),
        inst.purpose.label(),
        ancestry,
        if inst.cwd.is_empty() { "(unknown)" } else { &inst.cwd },
        if inst.cmdline.is_empty() { "(empty)" } else { &inst.cmdline },
        ports,
        children,
    )
}
