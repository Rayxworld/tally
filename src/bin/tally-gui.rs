use eframe::egui;
use std::path::PathBuf;
use std::time::SystemTime;
use tally::config::{Config, KnownDirs};
use tally::productivity::{
    batch_prefix, find_duplicates, folder_stats, move_duplicates, DuplicateGroup, FolderStats,
};
use tally::sweep::{plan_sweep, sweep, PlannedAction, PlannedActionKind, SweepOptions, SweepPlan};
use tally::undo;

struct TallyApp {
    folder: String,
    message: String,
    plan: Option<SweepPlan>,
    error: Option<String>,
    confirm_organize: bool,
    duplicates: Vec<DuplicateGroup>,
    stats: Option<FolderStats>,
    command_open: bool,
    command: String,
    rename_prefix: String,
}

impl Default for TallyApp {
    fn default() -> Self {
        Self {
            folder: String::new(),
            message: "Choose a folder to preview its organization.".to_owned(),
            plan: None,
            error: None,
            confirm_organize: false,
            duplicates: Vec::new(),
            stats: None,
            command_open: false,
            command: String::new(),
            rename_prefix: String::new(),
        }
    }
}

impl TallyApp {
    fn clear_preview(&mut self) {
        self.plan = None;
        self.error = None;
        self.confirm_organize = false;
    }

    fn preview(&mut self) {
        self.clear_preview();

        let target = PathBuf::from(self.folder.trim());
        if !target.is_dir() {
            self.error = Some("Choose an existing folder.".to_owned());
            return;
        }

        self.stats = folder_stats(&target).ok();

        let config_path = tally::config_path();
        let cfg = match Config::load(&config_path) {
            Ok(cfg) => cfg,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };

        let known = KnownDirs::detect();
        match plan_sweep(&cfg, &known, &target, SystemTime::now()) {
            Ok(plan) => {
                self.message = if plan.warning.is_some() {
                    "This folder is protected.".to_owned()
                } else {
                    "Preview ready. No files have been changed.".to_owned()
                };
                self.plan = Some(plan);
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn scan_duplicates(&mut self) {
        self.error = None;
        let target = PathBuf::from(self.folder.trim());
        if !target.is_dir() {
            self.error = Some("Choose an existing folder first.".into());
            return;
        }
        match find_duplicates(&target) {
            Ok(groups) => {
                let clones: usize = groups.iter().map(|g| g.files.len() - 1).sum();
                self.message = format!(
                    "Found {clones} exact clones in {} groups. Nothing moved yet.",
                    groups.len()
                );
                self.duplicates = groups;
                self.stats = folder_stats(&target).ok();
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn move_clones(&mut self) {
        let target = PathBuf::from(self.folder.trim());
        match move_duplicates(&self.duplicates, &target, &tally::open_ledger()) {
            Ok(count) => {
                self.message = format!("Moved {count} clones into _Duplicates. Undo is available.");
                self.duplicates.clear();
                self.stats = folder_stats(&target).ok();
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn undo_last(&mut self) {
        match undo::undo(&tally::open_ledger(), 1, false) {
            Ok(summary) => {
                self.message = format!("Undo complete: {} files restored.", summary.restored)
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn rename_previewed(&mut self) {
        let Some(plan) = &self.plan else {
            self.error =
                Some("Preview a folder first so Tally knows which files to rename.".into());
            return;
        };
        let files: Vec<PathBuf> = plan
            .actions
            .iter()
            .filter(|a| {
                matches!(
                    a.kind,
                    PlannedActionKind::Move | PlannedActionKind::Duplicate
                )
            })
            .map(|a| a.source.clone())
            .collect();
        match batch_prefix(&files, &self.rename_prefix, &tally::open_ledger()) {
            Ok(count) => {
                self.message = format!("Renamed {count} files. Undo is available.");
                self.preview();
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn organize(&mut self) {
        self.confirm_organize = false;
        self.error = None;

        let target = PathBuf::from(self.folder.trim());
        if !target.is_dir() {
            self.error = Some("The selected folder no longer exists.".to_owned());
            self.plan = None;
            return;
        }

        let cfg = match Config::load(&tally::config_path()) {
            Ok(cfg) => cfg,
            Err(error) => {
                self.error = Some(error);
                return;
            }
        };

        let known = KnownDirs::detect();
        let ledger = tally::open_ledger();

        let opts = SweepOptions {
            dry_run: false,
            verbose: true,
            now: SystemTime::now(),
        };

        match sweep(&cfg, &known, &target, &opts, &ledger) {
            Ok(summary) => {
                self.message = format!(
                    "Finished: {} moved, {} duplicates set aside, {} skipped, {} failed.",
                    summary.moved, summary.duplicates, summary.skipped, summary.failed
                );
                self.preview();
            }
            Err(error) => self.error = Some(error),
        }
    }
}

fn action_label(action: &PlannedAction) -> &'static str {
    match action.kind {
        PlannedActionKind::Move => "MOVE",
        PlannedActionKind::Duplicate => "DUPLICATE",
        PlannedActionKind::Skip => "SKIP",
        PlannedActionKind::Failed => "CHECK",
    }
}

impl eframe::App for TallyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let background = egui::Color32::from_rgb(13, 17, 27);
        let blue = egui::Color32::from_rgb(65, 135, 255);
        let white = egui::Color32::from_rgb(240, 244, 250);
        let muted = egui::Color32::from_rgb(155, 167, 185);
        let green = egui::Color32::from_rgb(105, 205, 155);
        let amber = egui::Color32::from_rgb(245, 190, 90);
        let red = egui::Color32::from_rgb(245, 115, 115);

        let mut browse_clicked = false;
        let mut preview_clicked = false;
        let mut organize_clicked = false;
        let mut scan_clicked = false;
        let mut move_clones_clicked = false;
        let mut undo_clicked = false;
        let mut rename_clicked = false;

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::K)) {
            self.command_open = true;
        }

        ctx.set_visuals(egui::Visuals::dark());

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(background).inner_margin(28.0))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("TALLY").size(27.0).strong().color(blue));
                    ui.add_space(12.0);
                    ui.label(
                        egui::RichText::new("File organization, under your control.")
                            .size(13.0)
                            .color(muted),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Batch rename previewed files:")
                            .size(11.0)
                            .color(muted),
                    );
                    ui.add(
                        egui::TextEdit::singleline(&mut self.rename_prefix)
                            .hint_text("Prefix, e.g. Trip2026_")
                            .desired_width(180.0),
                    );
                    rename_clicked = ui.button("Apply prefix").clicked();
                });

                ui.add_space(24.0);
                ui.label(
                    egui::RichText::new("Choose a folder")
                        .size(18.0)
                        .strong()
                        .color(white),
                );
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.folder)
                            .hint_text("Folder path")
                            .desired_width(ui.available_width() - 86.0),
                    );
                    browse_clicked = ui.button("Browse").clicked();
                });

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    scan_clicked = ui.button("Find clones").clicked();
                    undo_clicked = ui.button("Undo last").clicked();
                    ui.label(
                        egui::RichText::new("Ctrl+K commands")
                            .size(11.0)
                            .color(muted),
                    );
                });
                if let Some(stats) = &self.stats {
                    let top: u64 = stats.largest.iter().map(|(_, bytes)| *bytes).sum();
                    let pct = if stats.bytes == 0 {
                        0
                    } else {
                        top * 100 / stats.bytes
                    };
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "This folder: {} files · {} · top 3 use {pct}%",
                            stats.files,
                            human_bytes(stats.bytes)
                        ))
                        .color(muted),
                    );
                    for (path, bytes) in &stats.largest {
                        ui.label(
                            egui::RichText::new(format!(
                                "Space hog: {} ({})",
                                path.file_name().unwrap_or_default().to_string_lossy(),
                                human_bytes(*bytes)
                            ))
                            .size(11.0)
                            .color(amber),
                        );
                    }
                }
                let preview_button = egui::Button::new(
                    egui::RichText::new("Preview organization")
                        .strong()
                        .color(white),
                )
                .fill(blue)
                .min_size(egui::vec2(ui.available_width(), 40.0));

                preview_clicked = ui.add(preview_button).clicked();

                ui.add_space(10.0);
                ui.label(egui::RichText::new(&self.message).size(13.0).color(muted));

                if let Some(error) = &self.error {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(error).color(red));
                }

                if let Some(plan) = &self.plan {
                    ui.add_space(14.0);

                    if let Some(warning) = &plan.warning {
                        ui.label(egui::RichText::new(warning).strong().color(amber));
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{} to move", plan.summary.moved))
                                    .strong()
                                    .color(green),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} duplicates",
                                    plan.summary.duplicates
                                ))
                                .color(amber),
                            );
                            ui.label(
                                egui::RichText::new(format!("{} skipped", plan.summary.skipped))
                                    .color(muted),
                            );
                            if plan.summary.failed > 0 {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} need checking",
                                        plan.summary.failed
                                    ))
                                    .color(red),
                                );
                            }
                        });

                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .max_height(210.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                for action in &plan.actions {
                                    let color = match action.kind {
                                        PlannedActionKind::Move => green,
                                        PlannedActionKind::Duplicate => amber,
                                        PlannedActionKind::Skip => muted,
                                        PlannedActionKind::Failed => red,
                                    };

                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(action_label(action))
                                                .size(10.0)
                                                .strong()
                                                .color(color),
                                        );

                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new(
                                                    action
                                                        .source
                                                        .file_name()
                                                        .unwrap_or_default()
                                                        .to_string_lossy(),
                                                )
                                                .color(white),
                                            );

                                            if let Some(destination) = &action.destination {
                                                ui.label(
                                                    egui::RichText::new(format!(
                                                        "→ {}",
                                                        destination.display()
                                                    ))
                                                    .size(11.0)
                                                    .color(muted),
                                                );
                                            } else {
                                                ui.label(
                                                    egui::RichText::new(&action.reason)
                                                        .size(11.0)
                                                        .color(muted),
                                                );
                                            }
                                        });
                                    });
                                    ui.add_space(6.0);
                                }
                            });

                        ui.add_space(10.0);

                        let can_organize = plan.summary.moved + plan.summary.duplicates > 0;

                        if self.confirm_organize {
                            ui.label(
                                egui::RichText::new(
                                    "Confirm: Tally will move the listed eligible files.",
                                )
                                .strong()
                                .color(amber),
                            );

                            ui.horizontal(|ui| {
                                if ui.button("Cancel").clicked() {
                                    self.confirm_organize = false;
                                }

                                organize_clicked = ui.button("Yes, organize files").clicked();
                            });
                        } else {
                            let button = egui::Button::new(
                                egui::RichText::new("Organize files").strong().color(white),
                            )
                            .fill(green);

                            if ui.add_enabled(can_organize, button).clicked() {
                                self.confirm_organize = true;
                            }
                        }
                    }
                }

                if !self.duplicates.is_empty() {
                    ui.add_space(14.0);
                    let clones: usize = self.duplicates.iter().map(|g| g.files.len() - 1).sum();
                    let reclaim: u64 = self
                        .duplicates
                        .iter()
                        .map(|g| g.size * (g.files.len() as u64 - 1))
                        .sum();
                    ui.label(
                        egui::RichText::new(format!(
                            "Exact clones: {clones} files · {} recoverable",
                            human_bytes(reclaim)
                        ))
                        .strong()
                        .color(amber),
                    );
                    for group in self.duplicates.iter().take(6) {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} identical copies · {}",
                                group.files.len(),
                                group.files[0]
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                            ))
                            .size(11.0)
                            .color(muted),
                        );
                    }
                    move_clones_clicked = ui.button("Move clones to _Duplicates").clicked();
                }
            });

        if self.command_open {
            egui::Window::new("Command Palette")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.command)
                            .hint_text("Type: organize, clones, undo"),
                    );
                    if ui.button("Preview organization").clicked() {
                        preview_clicked = true;
                        self.command_open = false;
                    }
                    if ui.button("Find clones").clicked() {
                        scan_clicked = true;
                        self.command_open = false;
                    }
                    if ui.button("Undo last action").clicked() {
                        undo_clicked = true;
                        self.command_open = false;
                    }
                });
        }

        if browse_clicked {
            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                self.folder = path.display().to_string();
                self.message = "Folder selected. Preview before organizing.".to_owned();
                self.clear_preview();
            }
        }

        if preview_clicked {
            self.preview();
        }

        if scan_clicked {
            self.scan_duplicates();
        }
        if move_clones_clicked {
            self.move_clones();
        }
        if undo_clicked {
            self.undo_last();
        }
        if rename_clicked {
            self.rename_previewed();
        }

        if organize_clicked {
            self.organize();
        }
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([820.0, 680.0])
            .with_min_inner_size([620.0, 500.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Tally",
        options,
        Box::new(|_cc| Ok(Box::new(TallyApp::default()))),
    )
}
