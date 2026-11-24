//! The windows on top of the main one: scan summary (donut chart), Settings,
//! Fix to real resolution, confirmations and messages, drawn in the theme's
//! colours and the player's fonts and buttons (2.1).

use std::collections::HashSet;
use std::time::Instant;

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Pos2, RichText, Sense, Vec2};
use realhires::fix::{done_summary, plan_fix, skip_summary, Backup, FixOutcome, FixPlan, FIXABLE_VERDICTS};
use realhires::paths::{abspath, file_name};
use realhires::pyfmt::{f, pct};
use realhires::settings::{Kind, SValue, Settings, GROUPS};
use realhires::verdict::{verdict_breakdown, FileResult, Verdict, PROBLEM_VERDICTS, VERDICT_ORDER};

use crate::app::{arc, App};
use crate::look::{self, Palette};
use crate::style::{
    error_color, flat_button, heading_text, hex, ok_color, on_page, pal, panel_button, plural, primary_button,
    progress, reveal_in_file_manager, secondary, verdict_style, verdict_text, warn_color, window_heading,
};

pub enum ConfirmAction {
    ForgetSaved,
    FixAfterScan(Vec<FileResult>),
    FixDeleteOriginals,
    CloseFix,
    CloseLyrics,
}

pub struct Confirm {
    pub title: String,
    pub text: String,
    pub action: ConfirmAction,
}

pub fn show_all(app: &mut App, ctx: &egui::Context) {
    if let Some(mut w) = app.summary.take() {
        if w.show(app, ctx) {
            app.summary = Some(w);
        }
    }
    if let Some(mut w) = app.settings_win.take() {
        if w.show(app, ctx) {
            app.settings_win = Some(w);
        }
    }
    if let Some(mut w) = app.fix_win.take() {
        let keep = w.show(app, ctx);
        if keep {
            app.fix_win = Some(w);
        }
    }
    if let Some(mut w) = app.lyrics_win.take() {
        if w.show(app, ctx) {
            app.lyrics_win = Some(w);
        }
    }
    if let Some(mut w) = app.fix_done.take() {
        if w.show(ctx) {
            app.fix_done = Some(w);
        }
    }
    if let Some(c) = app.confirm.take() {
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("realhires-confirm")).show(ctx, |ui| {
            let pal = pal(ui);
            ui.set_max_width(460.0);
            ui.label(window_heading(&c.title, &pal));
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(&c.text).color(pal.text)).wrap());
            ui.add_space(12.0);
            crate::style::buttons_right(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if primary_button(ui, "Yes", true).clicked() {
                    answer = Some(true);
                }
                if flat_button(ui, "No", true).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    answer = Some(false);
                }
            });
        });
        match answer {
            None => app.confirm = Some(c),
            Some(false) => {}
            Some(true) => match c.action {
                ConfirmAction::ForgetSaved => app.forget_saved(),
                ConfirmAction::FixAfterScan(results) => app.open_fix(results, true),
                ConfirmAction::FixDeleteOriginals => {
                    if let Some(mut w) = app.fix_win.take() {
                        w.start(app);
                        app.fix_win = Some(w);
                    }
                }
                // keep it working while hidden, so the list is updated and the pop-up appears
                ConfirmAction::CloseFix => {
                    if let Some(w) = app.fix_win.as_mut() {
                        w.hidden = true;
                    }
                }
                ConfirmAction::CloseLyrics => app.lyrics_win = None, // dropping it stops the downloads
            },
        }
    }
    if let Some((title, text)) = app.info.take() {
        let mut close = false;
        egui::Modal::new(egui::Id::unique("realhires-info")).show(ctx, |ui| {
            let pal = pal(ui);
            ui.set_max_width(460.0);
            ui.label(window_heading(&title, &pal));
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(&text).color(pal.text)).wrap());
            ui.add_space(12.0);
            crate::style::buttons_right(ui, |ui| {
                if primary_button(ui, "OK", true).clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))
                {
                    close = true;
                }
            });
        });
        if !close {
            app.info = Some((title, text));
        }
    }
}

// ----------------------------------------------------------------- summary ---

pub struct SummaryWindow {
    breakdown: Vec<(Verdict, usize, f64)>,
    total: usize,
    problems: usize,
    elapsed: Option<f64>,
    again: bool,
}

impl SummaryWindow {
    pub fn new(app: &App) -> SummaryWindow {
        let results: Vec<&FileResult> = app.results.values().collect();
        SummaryWindow {
            breakdown: verdict_breakdown(results.iter().copied()),
            total: results.len(),
            problems: results.iter().filter(|r| r.is_problem()).count(),
            elapsed: app.last_elapsed,
            again: app.cfg.show_summary_popup,
        }
    }

    fn show(&mut self, app: &mut App, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut close: Option<Option<Vec<Verdict>>> = None;
        egui::Window::new("Scan summary")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let pal = pal(ui);
                let n = self.total;
                let mut head = format!("Scanned {}", plural(n, "file"));
                if let Some(e) = self.elapsed {
                    head += &format!(" in {} s", f(e, 1));
                }
                ui.label(window_heading(&head, &pal));
                let pct_problems = if n > 0 { 100.0 * self.problems as f64 / n as f64 } else { 0.0 };
                if self.problems > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} ({} %) are fake or suspicious.",
                            plural(self.problems, "file"),
                            f(pct_problems, 1)
                        ))
                        .color(on_page(hex(0xa3122a), &pal)),
                    );
                } else {
                    ui.label(RichText::new("No fake or suspicious files found.").color(ok_color(&pal)));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(v) = self.donut(ui, pct_problems, &pal) {
                        close = Some(Some(vec![v]));
                    }
                    ui.add_space(14.0);
                    egui::Grid::new("legend").num_columns(4).spacing([8.0, 2.0]).show(ui, |ui| {
                        for (v, count, p) in &self.breakdown {
                            let (label, color, desc) = verdict_style(*v);
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 5.5, color);
                            let name = ui.add(
                                egui::Label::new(
                                    RichText::new(label).font(look::title(13.0)).color(verdict_text(*v, &pal)),
                                )
                                .sense(Sense::click()),
                            );
                            if name.clicked() {
                                close = Some(Some(vec![*v]));
                            }
                            name.on_hover_cursor(egui::CursorIcon::PointingHand);
                            ui.label(
                                RichText::new(format!("{:>5} %", f(*p, 1))).font(look::title(13.0)).color(pal.text),
                            );
                            ui.label(secondary(plural(*count, "file"), &pal));
                            ui.end_row();
                            ui.label("");
                            ui.label(RichText::new(desc).font(look::text(11.5)).color(pal.secondary));
                            ui.end_row();
                        }
                    });
                });
                ui.label(
                    RichText::new("Click a slice or a name to list only those files.")
                        .font(look::text(11.5))
                        .color(pal.secondary),
                );
                ui.checkbox(&mut self.again, "Show this summary after every scan");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if flat_button(ui, "Show problem files", true).clicked() {
                        close = Some(Some(PROBLEM_VERDICTS.to_vec()));
                    }
                    if flat_button(ui, "Show all files", true).clicked() {
                        close = Some(Some(VERDICT_ORDER.to_vec()));
                    }
                    if flat_button(ui, "Export CSV…", true).clicked() {
                        app.export(false, true);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, "Close", true).clicked() {
                            close = Some(None);
                        }
                    });
                });
            });
        if !open && close.is_none() {
            close = Some(None);
        }
        match close {
            None => true,
            Some(filters) => {
                if self.again != app.cfg.show_summary_popup {
                    app.cfg.show_summary_popup = self.again;
                    app.save_settings();
                }
                if let Some(vs) = filters {
                    app.set_filters(&vs);
                }
                false
            }
        }
    }

    /// The donut chart; returns the verdict of a clicked slice.
    fn donut(&self, ui: &mut egui::Ui, pct_problems: f64, pal: &Palette) -> Option<Verdict> {
        let size = 260.0f32;
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
        let p = ui.painter_at(rect);
        let c = rect.center();
        let (outer, hole) = (size / 2.0 - 8.0, size * 0.27);
        let total = self.total.max(1) as f32;
        let mut start = 0.0f32;
        let mut spans = vec![];
        for (v, count, pc) in &self.breakdown {
            let ext = *count as f32 / total * std::f32::consts::TAU;
            arc(&p, c, (outer + hole) / 2.0, start, start + ext, outer - hole, verdict_style(*v).1);
            if *pc >= 7.0 {
                let mid = start + ext / 2.0;
                let r = outer * 0.78;
                p.text(
                    Pos2::new(c.x + r * mid.sin(), c.y - r * mid.cos()),
                    Align2::CENTER_CENTER,
                    format!("{}%", f(*pc, 0)),
                    look::title(12.0),
                    Color32::WHITE,
                );
            }
            spans.push((*v, start, start + ext));
            start += ext;
        }
        let color = if pct_problems > 0.0 { on_page(hex(0xa3122a), pal) } else { ok_color(pal) };
        p.text(
            c - Vec2::new(0.0, 12.0),
            Align2::CENTER_CENTER,
            format!("{}%", f(pct_problems, 0)),
            look::title(26.0),
            color,
        );
        p.text(c + Vec2::new(0.0, 16.0), Align2::CENTER_CENTER, "fake or suspicious", look::text(11.0), pal.secondary);
        let hit = |pos: Pos2| -> Option<Verdict> {
            let d = pos - c;
            let r = d.length();
            if r < hole || r > outer {
                return None;
            }
            let a = d.x.atan2(-d.y).rem_euclid(std::f32::consts::TAU);
            spans.iter().find(|(_, a0, a1)| a >= *a0 && a < *a1).map(|(v, _, _)| *v)
        };
        if let Some(pos) = resp.hover_pos() {
            if hit(pos).is_some() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        if resp.clicked() {
            return resp.interact_pointer_pos().and_then(hit);
        }
        None
    }
}

// ---------------------------------------------------------------- settings ---

pub struct SettingsWindow {
    draft: Settings,
    tab: usize,
    error: Option<String>,
}

impl SettingsWindow {
    pub fn new(cfg: &Settings) -> SettingsWindow {
        SettingsWindow { draft: cfg.clone(), tab: 0, error: None }
    }

    fn show(&mut self, app: &mut App, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut result: Option<bool> = None;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .default_width(780.0)
            .resizable(true)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                let pal = pal(ui);
                ui.horizontal_top(|ui| {
                    // the groups on the left, as on the player's Settings page
                    ui.vertical(|ui| {
                        ui.set_width(170.0);
                        for (i, g) in GROUPS.iter().enumerate() {
                            let (rect, resp) = ui.allocate_exact_size(vec2(170.0, 30.0), Sense::click());
                            let sel = self.tab == i;
                            if sel {
                                ui.painter().rect_filled(rect, CornerRadius::same(6), pal.accent);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(6), pal.hover);
                            }
                            let color = if sel { Color32::WHITE } else { pal.text };
                            ui.painter().text(
                                pos2(rect.left() + 10.0, rect.center().y),
                                Align2::LEFT_CENTER,
                                *g,
                                look::text(13.0),
                                color,
                            );
                            if resp.clicked() {
                                self.tab = i;
                            }
                        }
                    });
                    ui.add_space(14.0);
                    ui.vertical(|ui| {
                        let group = GROUPS[self.tab];
                        ui.label(RichText::new(group).font(look::title(18.0)).color(pal.text));
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().max_height(430.0).show(ui, |ui| {
                            egui::Grid::new("settings").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                                for info in Settings::field_info().into_iter().filter(|i| i.group == group) {
                                    if let Some(e) = setting_row(ui, &info, &mut self.draft, &app.choices) {
                                        self.error = Some(e);
                                    }
                                }
                            });
                            if group == "Saved results" {
                                ui.add_space(8.0);
                                let text = match &app.store {
                                    Some(st) => {
                                        let s = st.stats();
                                        format!(
                                            "{} saved for {} in {} ({} MB, of which graphs {} MB).",
                                            plural(s.results as usize, "result"),
                                            plural(s.files as usize, "file"),
                                            s.path.display(),
                                            f(s.disk_bytes as f64 / 1e6, 1),
                                            f(s.graph_bytes as f64 / 1e6, 1)
                                        )
                                    }
                                    None => {
                                        "Saving results is turned off, or the saved results cannot be opened.".into()
                                    }
                                };
                                ui.add(egui::Label::new(secondary(text, &pal)).wrap());
                                ui.add_space(4.0);
                                if flat_button(ui, "Delete all saved results…", true).clicked() {
                                    app.forget_saved_ask();
                                }
                            }
                        });
                    });
                });
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).color(error_color(&pal)));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if flat_button(ui, "Restore defaults", true).clicked() {
                        self.draft = Settings::default();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if primary_button(ui, "Save", true).clicked() {
                            result = Some(true);
                        }
                        if flat_button(ui, "Cancel", true).clicked() {
                            result = Some(false);
                        }
                    });
                });
            });
        match (open, result) {
            (false, _) | (_, Some(false)) => false,
            (_, Some(true)) => {
                app.apply_settings(self.draft.clone());
                false
            }
            _ => true,
        }
    }
}

/// The label and widget of one setting in a two-column grid, shared by the
/// Settings window and the player's Settings page (2.0).  `choices` holds the
/// menus that depend on the computer (installed fonts, output devices), each
/// with "" (the default) first.  Returns an error message for a bad value.
pub fn setting_row(
    ui: &mut egui::Ui,
    info: &realhires::settings::FieldInfo,
    draft: &mut Settings,
    choices: &std::collections::HashMap<&'static str, Vec<String>>,
) -> Option<String> {
    ui.label(info.label).on_hover_text(&info.help);
    let cur = draft.get(info.name).unwrap();
    let mut new = cur.clone();
    let resp = match &mut new {
        SValue::Float(x) => {
            let (lo, hi) = (info.min.unwrap_or(f64::MIN), info.max.unwrap_or(f64::MAX));
            let speed = ((hi - lo) / 500.0).clamp(0.001, 10.0);
            ui.add(egui::DragValue::new(x).range(lo..=hi).speed(speed).max_decimals(3))
        }
        SValue::Int(x) => {
            let (lo, hi) = (info.min.unwrap_or(0.0) as i64, info.max.unwrap_or(1e9) as i64);
            ui.add(egui::DragValue::new(x).range(lo..=hi))
        }
        SValue::Bool(b) => ui.checkbox(b, ""),
        SValue::Str(s) if info.kind == Kind::Choice => {
            let width = choice_width(ui, info.choices.iter().copied());
            egui::ComboBox::from_id_salt(info.name)
                .selected_text(s.clone())
                .width(width)
                .show_ui(ui, |ui| {
                    for c in info.choices {
                        ui.selectable_value(s, c.to_string(), *c);
                    }
                })
                .response
        }
        SValue::Str(s) if choices.contains_key(info.name) => {
            let list = &choices[info.name];
            let label = |v: &str| -> String {
                if !v.is_empty() {
                    v.to_string()
                } else if info.name == "font_family" {
                    "SF Pro (macOS) / Inter".to_string()
                } else {
                    "System default".to_string()
                }
            };
            let width = choice_width(ui, list.iter().map(|v| v.as_str()).chain(["SF Pro (macOS) / Inter"])).min(360.0);
            egui::ComboBox::from_id_salt(info.name)
                .selected_text(label(s))
                .width(width)
                .height(360.0)
                .show_ui(ui, |ui| {
                    let mut listed = list.clone();
                    if !s.is_empty() && !listed.iter().any(|v| v == s) {
                        listed.push(s.clone()); // a saved choice that is not there now
                    }
                    for v in &listed {
                        ui.selectable_value(s, v.clone(), label(v));
                    }
                })
                .response
        }
        SValue::Str(s) => ui.add(egui::TextEdit::singleline(s).desired_width(360.0)),
        SValue::List(l) if info.kind == Kind::Paths => {
            ui.vertical(|ui| {
                let mut remove = None;
                for (i, folder) in l.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.small_button("Remove").clicked() {
                            remove = Some(i);
                        }
                        ui.scope(|ui| {
                            ui.set_max_width(460.0);
                            ui.add(egui::Label::new(folder.as_str()).truncate()).on_hover_text(folder.as_str());
                        });
                    });
                }
                if l.is_empty() {
                    ui.weak("No folder yet.");
                }
                if let Some(i) = remove {
                    l.remove(i);
                }
                if ui.button("Add folder…").clicked() {
                    if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                        let d = dir.to_string_lossy().into_owned();
                        if !l.contains(&d) {
                            l.push(d);
                        }
                    }
                }
            })
            .response
        }
        SValue::List(l) => {
            ui.vertical(|ui| {
                for c in info.choices {
                    let mut on = l.iter().any(|x| x == c);
                    if ui.checkbox(&mut on, *c).changed() {
                        if on {
                            l.push(c.to_string());
                        } else {
                            l.retain(|x| x != c);
                        }
                    }
                }
            })
            .response
        }
    };
    resp.on_hover_text(&info.help);
    let mut error = None;
    if new != cur {
        if let Err(e) = draft.set(info.name, new) {
            error = Some(e);
        }
    }
    ui.end_row();
    error
}

/// A drop-down as wide as its longest choice.
fn choice_width<'a>(ui: &egui::Ui, items: impl Iterator<Item = &'a str>) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let widest = items
        .map(|c| ui.ctx().fonts_mut(|f| f.layout_no_wrap(c.to_string(), font.clone(), egui::Color32::WHITE).size().x))
        .fold(120.0f32, f32::max);
    widest + 40.0
}

// --------------------------------------------------------------------- fix ---

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Ready,
    Skip,
    Ok,
    Fail,
}

struct FixRow {
    path: String,
    name: String,
    change: String,
    status: String,
    output: String,
    kind: RowKind,
}

pub struct FixWindow {
    results: Vec<FileResult>,
    output: String,
    backup: String,
    backup_dir: String,
    folder: String,
    format: String,
    bits: String,
    verify: bool,
    plans: Vec<FixPlan>,
    rows: Vec<FixRow>,
    head: String,
    note: String,
    running: bool,
    started: bool,
    done: usize,
    pub status: String,
    autostart: Option<Instant>,
    done_paths: Vec<String>,
    /// FixOutcome of every file of the last run
    pub outcomes: Vec<FixOutcome>,
    /// fake files left alone on purpose
    pub skipped: usize,
    /// closed while fixing: finishes in the background
    pub hidden: bool,
}

fn choices(name: &str) -> &'static [&'static str] {
    Settings::field(name).map(|f| f.choices).unwrap_or(&[])
}

impl FixWindow {
    pub fn new(cfg: &Settings, results: Vec<FileResult>, autostart: bool) -> FixWindow {
        let mut w = FixWindow {
            results,
            output: cfg.fix_output.clone(),
            backup: cfg.fix_backup.clone(),
            backup_dir: cfg.fix_backup_folder.clone(),
            folder: cfg.fix_folder.clone(),
            format: cfg.fix_format.clone(),
            bits: cfg.fix_bits.clone(),
            verify: cfg.fix_verify,
            plans: vec![],
            rows: vec![],
            head: String::new(),
            note: String::new(),
            running: false,
            started: false,
            done: 0,
            status: String::new(),
            autostart: None,
            done_paths: vec![],
            outcomes: vec![],
            skipped: 0,
            hidden: false,
        };
        w.replan(cfg);
        if autostart && !w.plans.is_empty() {
            w.autostart = Some(Instant::now());
        }
        w
    }

    fn cfg(&self, base: &Settings) -> Settings {
        let mut c = base.clone();
        c.fix_output = self.output.clone();
        c.fix_folder = self.folder.clone();
        c.fix_backup = self.backup.clone();
        c.fix_backup_folder = self.backup_dir.clone();
        c.fix_format = self.format.clone();
        c.fix_bits = self.bits.clone();
        c.fix_verify = self.verify;
        c
    }

    fn replan(&mut self, base: &Settings) {
        if self.running || self.started {
            return;
        }
        let cfg = self.cfg(base);
        let into = cfg.fix_output.ends_with("into a folder");
        let replace = cfg.fix_output.starts_with("Replace");
        self.plans.clear();
        self.rows.clear();
        let mut notes: Vec<String> = vec![];
        let mut skipped = vec![];
        for r in &self.results {
            match plan_fix(r, &cfg) {
                Ok(plan) => {
                    for n in &plan.notes {
                        if !notes.contains(n) {
                            notes.push(n.clone());
                        }
                    }
                    let out = if plan.replace_original {
                        let same = abspath(&plan.output) == abspath(&plan.source);
                        let mut o = if same {
                            "Same file".to_string()
                        } else {
                            format!("Replaced by {}", file_name(&plan.output))
                        };
                        o += match plan.backup {
                            Backup::Folder => " (original → backup folder)",
                            Backup::Bak => " (original → .bak)",
                            Backup::None => " (original deleted)",
                        };
                        o
                    } else if into {
                        plan.output.clone()
                    } else {
                        format!("Copy: {}", file_name(&plan.output))
                    };
                    self.rows.push(FixRow {
                        path: r.path.clone(),
                        name: file_name(&r.path),
                        change: plan.description.clone(),
                        status: "Ready".into(),
                        output: out,
                        kind: RowKind::Ready,
                    });
                    self.plans.push(plan);
                }
                Err(why) => skipped.push((r.clone(), why)),
            }
        }
        for (r, why) in &skipped {
            self.rows.push(FixRow {
                path: r.path.clone(),
                name: file_name(&r.path),
                change: r.claimed(),
                status: why.clone(),
                output: String::new(),
                kind: RowKind::Skip,
            });
        }
        notes.sort();
        self.skipped = skipped.len();
        let n = self.plans.len();
        let verb = if replace { "Fix" } else { "Save corrected copies of" };
        self.head =
            if n > 0 { format!("{verb} {}: real resolution", plural(n, "file")) } else { "Nothing to fix".into() };
        let mut why_not = skip_summary(&skipped);
        if !skipped.is_empty() {
            why_not += " They are listed last, in grey.";
        }
        let mut lines = vec![];
        if !why_not.is_empty() {
            lines.push(why_not);
        }
        lines.extend(notes);
        self.note = lines.join("\n");
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn set_status(&mut self, path: &str, text: String, kind: Option<RowKind>) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.path == path) {
            r.status = text;
            if let Some(k) = kind {
                r.kind = k;
            }
        }
    }

    pub fn done(&mut self, o: FixOutcome) {
        self.done += 1;
        self.outcomes.push(o.clone());
        let src = o.plan.source.clone();
        if o.ok {
            let mut txt = if o.plan.replace_original { "Fixed".to_string() } else { "Copy saved".to_string() };
            if let Some(n) = &o.new_result {
                txt += &format!(": now {} ({})", n.claimed(), verdict_style(n.verdict).0);
            }
            txt += &format!(", {} of the size", pct(o.bytes_after as f64 / o.bytes_before.max(1) as f64, 0));
            if let (false, Some(out)) = (o.plan.replace_original, &o.output) {
                if let Some(r) = self.rows.iter_mut().find(|r| r.path == src) {
                    r.output = out.clone();
                }
            }
            if let Some(out) = &o.output {
                self.done_paths.push(out.clone());
            }
            self.set_status(&src, txt, Some(RowKind::Ok));
        } else {
            self.set_status(&src, o.message.clone(), Some(RowKind::Fail));
        }
    }

    pub fn finished(&mut self) {
        self.running = false;
        let ok = self.rows.iter().filter(|r| r.kind == RowKind::Ok).count();
        let bad = self.rows.iter().filter(|r| r.kind == RowKind::Fail).count();
        self.status = format!("{ok} fixed{}.", if bad > 0 { format!(", {bad} failed") } else { String::new() });
    }

    fn ask_start(&mut self, app: &mut App) {
        if self.plans.is_empty() || self.running || self.started {
            return;
        }
        if self.output.starts_with("Replace") && self.backup.starts_with("Delete") {
            app.confirm = Some(Confirm {
                title: "Delete originals?".into(),
                text: format!(
                    "Replace {} file(s) and delete the originals?\n\nThere will be no backup. The converted files contain everything the originals really had, but the originals cannot be restored.",
                    self.plans.len()
                ),
                action: ConfirmAction::FixDeleteOriginals,
            });
            return;
        }
        self.start(app);
    }

    pub fn start(&mut self, app: &mut App) {
        if self.plans.is_empty() || self.running || self.started {
            return;
        }
        let cfg = self.cfg(&app.cfg);
        app.cfg.fix_output = cfg.fix_output.clone();
        app.cfg.fix_folder = cfg.fix_folder.clone();
        app.cfg.fix_backup = cfg.fix_backup.clone();
        app.cfg.fix_backup_folder = cfg.fix_backup_folder.clone();
        app.cfg.fix_format = cfg.fix_format.clone();
        app.cfg.fix_bits = cfg.fix_bits.clone();
        app.cfg.fix_verify = cfg.fix_verify;
        app.save_settings();
        self.running = true;
        self.started = true;
        self.outcomes.clear();
        self.status = "Fixing…".into();
        let paths: Vec<String> = self.plans.iter().map(|p| p.source.clone()).collect();
        for p in paths {
            self.set_status(&p, "Waiting…".into(), None);
        }
        let mut seen = HashSet::new();
        let mut everything: Vec<FileResult> = vec![];
        for r in app.results.values().chain(&self.results) {
            if seen.insert(r.path.clone()) {
                everything.push(r.clone());
            }
        }
        crate::work::start_fix(
            self.plans.clone(),
            everything,
            cfg.clone(),
            realhires::effective_jobs(cfg.jobs),
            app.post.clone(),
        );
    }

    fn show(&mut self, app: &mut App, ctx: &egui::Context) -> bool {
        if self.hidden {
            return true;
        }
        if self.autostart.is_some_and(|t| t.elapsed().as_millis() >= 400) {
            self.autostart = None;
            self.ask_start(app);
        } else if self.autostart.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut open = true;
        let mut close = false;
        let before = (
            self.output.clone(),
            self.backup.clone(),
            self.backup_dir.clone(),
            self.folder.clone(),
            self.format.clone(),
            self.bits.clone(),
        );
        egui::Window::new("Fix to real resolution").open(&mut open).collapsible(false).default_width(900.0).resizable(true).pivot(Align2::CENTER_CENTER).default_pos(ctx.content_rect().center()).show(ctx, |ui| {
            let pal = pal(ui);
            ui.label(window_heading(&self.head, &pal));
            ui.add_space(2.0);
            ui.add(egui::Label::new(secondary(
                "Each file is rewritten at the sample rate and bit depth it really contains, so its properties show the actual values (e.g. a fake 24/96 upsampled from a CD becomes a genuine 16/44.1 file). It keeps its name, tags and cover art, nothing audible is lost and it gets smaller. (Only editing the numbers in the file would make it play at the wrong speed, so the audio itself is converted.)",
                &pal,
            )).wrap());
            ui.add_space(8.0);
            egui::ScrollArea::vertical().max_height(220.0).id_salt("fixrows").show(ui, |ui| {
                egui::Grid::new("fixgrid").num_columns(4).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
                    for h in ["File", "Claimed → real", "Status", "Result"] {
                        ui.label(heading_text(h, &pal));
                    }
                    ui.end_row();
                    for r in &self.rows {
                        let color = match r.kind {
                            RowKind::Skip => pal.secondary,
                            RowKind::Ok => ok_color(&pal),
                            RowKind::Fail => error_color(&pal),
                            RowKind::Ready => pal.text,
                        };
                        ui.label(RichText::new(&r.name).color(color));
                        ui.label(RichText::new(&r.change).color(color));
                        ui.allocate_ui_with_layout(Vec2::new(300.0, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.set_width(300.0);
                            ui.add(egui::Label::new(RichText::new(&r.status).color(color)).truncate()).on_hover_text(&r.status);
                        });
                        ui.label(RichText::new(&r.output).color(color));
                        ui.end_row();
                    }
                });
            });
            ui.add_space(8.0);
            let editable = !self.started;
            ui.add_enabled_ui(editable, |ui| {
                egui::Frame::new().fill(pal.lcd).corner_radius(10).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::Grid::new("fixopts").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        let combo = |ui: &mut egui::Ui, label: &str, id: &str, v: &mut String| {
                            ui.label(label);
                            egui::ComboBox::from_id_salt(id).selected_text(v.clone()).width(330.0).show_ui(ui, |ui| {
                                for c in choices(id) {
                                    ui.selectable_value(v, c.to_string(), *c);
                                }
                            });
                            ui.end_row();
                        };
                        combo(ui, "Fix files by:", "fix_output", &mut self.output);
                        combo(ui, "Original file:", "fix_backup", &mut self.backup);
                        let backup_folder = self.output.starts_with("Replace") && self.backup.starts_with("Move");
                        ui.label("Backup folder:");
                        ui.add_enabled_ui(backup_folder, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.backup_dir).desired_width(300.0));
                                if panel_button(ui, "Browse…", backup_folder).clicked() {
                                    if let Some(d) = rfd::FileDialog::new().set_title("Folder for backups of originals").pick_folder() {
                                        self.backup_dir = d.to_string_lossy().into_owned();
                                    }
                                }
                            });
                        });
                        ui.end_row();
                        let into = self.output.ends_with("into a folder");
                        ui.label("Copies folder:");
                        ui.add_enabled_ui(into, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut self.folder).desired_width(300.0));
                                if panel_button(ui, "Browse…", into).clicked() {
                                    if let Some(d) = rfd::FileDialog::new().set_title("Folder for corrected copies").pick_folder() {
                                        self.folder = d.to_string_lossy().into_owned();
                                    }
                                }
                            });
                        });
                        ui.end_row();
                        combo(ui, "Output format:", "fix_format", &mut self.format);
                        combo(ui, "Target bit depth:", "fix_bits", &mut self.bits);
                        ui.label("");
                        ui.checkbox(&mut self.verify, "Re-check each fixed file");
                        ui.end_row();
                        ui.label("");
                        ui.label(RichText::new("Empty backup folder = a 'Real-Hi-Res backups' folder next to each file (never scanned).").font(look::text(11.5)).color(pal.secondary));
                        ui.end_row();
                    });
                });
            });
            if !self.note.is_empty() {
                ui.add_space(8.0);
                ui.add(egui::Label::new(RichText::new(&self.note).color(warn_color(&pal))).wrap());
            }
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let n = self.plans.len().max(1);
                progress(ui, self.done as f32 / n as f32, 200.0);
                ui.label(RichText::new(&self.status).color(pal.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if flat_button(ui, "Close", true).clicked() {
                        close = true;
                    }
                    if primary_button(ui, "Fix files", !self.plans.is_empty() && !self.started).clicked() {
                        self.ask_start(app);
                    }
                    if flat_button(ui, "Show in folder", !self.done_paths.is_empty()).clicked() {
                        reveal_in_file_manager(&self.done_paths[0]);
                    }
                });
            });
        });
        let after = (
            self.output.clone(),
            self.backup.clone(),
            self.backup_dir.clone(),
            self.folder.clone(),
            self.format.clone(),
            self.bits.clone(),
        );
        if before != after {
            let base = app.cfg.clone();
            self.replan(&base);
        }
        if !open || close {
            if self.running {
                app.confirm = Some(Confirm {
                    title: "Conversion running".into(),
                    text:
                        "Conversions are still running. Close anyway? The files are finished in the background, and a \
                           pop-up tells you when they are done."
                            .into(),
                    action: ConfirmAction::CloseFix,
                });
                return true;
            }
            return false;
        }
        let _ = FIXABLE_VERDICTS;
        true
    }
}

// ---------------------------------------------------------- fixing finished ---

/// Pop-up shown when "Fix to real resolution" has finished: how many files
/// were fixed, how much smaller they are, where the originals went, what the
/// re-check found, and what failed.  It is not modal and stays when the Fix
/// window is closed.
pub struct FixDoneWindow {
    pub headline: String,
    pub lines: Vec<String>,
    failed: bool,
    outputs: Vec<String>,
}

impl FixDoneWindow {
    pub fn new(outcomes: &[FixOutcome], skipped: usize) -> FixDoneWindow {
        let label = |v: Verdict| {
            let name = verdict_style(v).0;
            let mut c = name.chars();
            match (c.next(), c.next()) {
                (Some(a), Some(b)) if b.is_lowercase() => format!("{}{}", a.to_lowercase(), &name[a.len_utf8()..]),
                _ => name.to_string(),
            }
        };
        let (headline, lines) = done_summary(outcomes, skipped, &label);
        FixDoneWindow {
            headline,
            lines,
            failed: outcomes.iter().any(|o| !o.ok),
            outputs: outcomes.iter().filter(|o| o.ok).filter_map(|o| o.output.clone()).collect(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true;
        let mut close = false;
        egui::Window::new("Fixing finished")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::new(0.0, -60.0))
            .show(ctx, |ui| {
                let pal = pal(ui);
                ui.set_max_width(460.0);
                let color =
                    if self.failed || self.outputs.is_empty() { on_page(hex(0xa3122a), &pal) } else { ok_color(&pal) };
                ui.label(RichText::new(&self.headline).font(look::title(16.0)).color(color));
                for (i, line) in self.lines.iter().enumerate() {
                    ui.add_space(if i == 0 { 8.0 } else { 4.0 });
                    ui.add(egui::Label::new(RichText::new(line).color(pal.text)).wrap());
                }
                ui.add_space(14.0);
                crate::style::buttons_right(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    if primary_button(ui, "Close", true).clicked() {
                        close = true;
                    }
                    if flat_button(ui, "Show in folder", !self.outputs.is_empty()).clicked() {
                        reveal_in_file_manager(&self.outputs[0]);
                    }
                });
            });
        open && !close
    }
}
