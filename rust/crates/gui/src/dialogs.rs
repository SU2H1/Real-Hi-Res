//! The windows on top of the main one: scan summary (donut chart), Settings,
//! Fix to real resolution, confirmations and messages.

use std::collections::HashSet;
use std::time::Instant;

use egui::{Align2, Color32, FontId, Pos2, RichText, Sense, Vec2};
use realhires::fix::{plan_fix, skip_summary, Backup, FixOutcome, FixPlan, FIXABLE_VERDICTS};
use realhires::paths::{abspath, file_name};
use realhires::pyfmt::{f, pct};
use realhires::settings::{Kind, SValue, Settings, GROUPS};
use realhires::verdict::{verdict_breakdown, FileResult, Verdict, PROBLEM_VERDICTS, VERDICT_ORDER};

use crate::app::{arc, App};
use crate::style::{hex, plural, reveal_in_file_manager, verdict_style};

pub enum ConfirmAction {
    ForgetSaved,
    FixAfterScan(Vec<FileResult>),
    FixDeleteOriginals,
    CloseFix,
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
    if let Some(c) = app.confirm.take() {
        let mut answer = None;
        egui::Modal::new(egui::Id::unique("realhires-confirm")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(&c.title);
            ui.add_space(6.0);
            ui.label(&c.text);
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Yes").clicked() {
                    answer = Some(true);
                }
                if ui.button("No").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
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
                ConfirmAction::CloseFix => app.fix_win = None,
            },
        }
    }
    if let Some((title, text)) = app.info.take() {
        let mut close = false;
        egui::Modal::new(egui::Id::unique("realhires-info")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading(&title);
            ui.add_space(6.0);
            ui.label(&text);
            ui.add_space(10.0);
            if ui.button("OK").clicked()
                || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape))
            {
                close = true;
            }
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
                let n = self.total;
                let mut head = format!("Scanned {}", plural(n, "file"));
                if let Some(e) = self.elapsed {
                    head += &format!(" in {} s", f(e, 1));
                }
                ui.label(RichText::new(head).strong().size(16.0));
                let pct_problems = if n > 0 { 100.0 * self.problems as f64 / n as f64 } else { 0.0 };
                if self.problems > 0 {
                    ui.label(
                        RichText::new(format!(
                            "{} ({} %) are fake or suspicious.",
                            plural(self.problems, "file"),
                            f(pct_problems, 1)
                        ))
                        .color(hex(0xa3122a)),
                    );
                } else {
                    ui.label(RichText::new("No fake or suspicious files found.").color(hex(0x2e7d32)));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if let Some(v) = self.donut(ui, pct_problems) {
                        close = Some(Some(vec![v]));
                    }
                    ui.add_space(14.0);
                    egui::Grid::new("legend").num_columns(4).spacing([8.0, 2.0]).show(ui, |ui| {
                        for (v, count, p) in &self.breakdown {
                            let (label, color, desc) = verdict_style(*v);
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                            ui.painter().rect_filled(r, 0.0, color);
                            let name = ui.add(
                                egui::Label::new(RichText::new(label).color(color).strong()).sense(Sense::click()),
                            );
                            if name.clicked() {
                                close = Some(Some(vec![*v]));
                            }
                            name.on_hover_cursor(egui::CursorIcon::PointingHand);
                            ui.label(RichText::new(format!("{:>5} %", f(*p, 1))).strong());
                            ui.label(RichText::new(plural(*count, "file")).color(hex(0x555555)));
                            ui.end_row();
                            ui.label("");
                            ui.label(RichText::new(desc).small().color(hex(0x777777)));
                            ui.end_row();
                        }
                    });
                });
                ui.label(
                    RichText::new("Click a slice or a name to list only those files.").small().color(hex(0x777777)),
                );
                ui.checkbox(&mut self.again, "Show this summary after every scan");
                ui.horizontal(|ui| {
                    if ui.button("Show problem files").clicked() {
                        close = Some(Some(PROBLEM_VERDICTS.to_vec()));
                    }
                    if ui.button("Show all files").clicked() {
                        close = Some(Some(VERDICT_ORDER.to_vec()));
                    }
                    if ui.button("Export CSV…").clicked() {
                        app.export(false, true);
                    }
                    if ui.button("Close").clicked() {
                        close = Some(None);
                    }
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
    fn donut(&self, ui: &mut egui::Ui, pct_problems: f64) -> Option<Verdict> {
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
                    FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
            spans.push((*v, start, start + ext));
            start += ext;
        }
        let color = if pct_problems > 0.0 { hex(0xa3122a) } else { hex(0x2e7d32) };
        p.text(
            c - Vec2::new(0.0, 12.0),
            Align2::CENTER_CENTER,
            format!("{}%", f(pct_problems, 0)),
            FontId::proportional(26.0),
            color,
        );
        p.text(
            c + Vec2::new(0.0, 16.0),
            Align2::CENTER_CENTER,
            "fake or suspicious",
            FontId::proportional(11.0),
            hex(0x555555),
        );
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
            .default_width(640.0)
            .resizable(true)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for (i, g) in GROUPS.iter().enumerate() {
                        if ui.add(egui::Button::selectable(self.tab == i, *g)).clicked() {
                            self.tab = i;
                        }
                    }
                });
                ui.separator();
                let group = GROUPS[self.tab];
                egui::ScrollArea::vertical().max_height(430.0).show(ui, |ui| {
                    egui::Grid::new("settings").num_columns(2).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
                        for info in Settings::field_info().into_iter().filter(|i| i.group == group) {
                            ui.label(info.label).on_hover_text(&info.help);
                            let cur = self.draft.get(info.name).unwrap();
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
                                    egui::ComboBox::from_id_salt(info.name)
                                        .selected_text(s.clone())
                                        .width(320.0)
                                        .show_ui(ui, |ui| {
                                            for c in info.choices {
                                                ui.selectable_value(s, c.to_string(), *c);
                                            }
                                        })
                                        .response
                                }
                                SValue::Str(s) => ui.add(egui::TextEdit::singleline(s).desired_width(360.0)),
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
                            if new != cur {
                                if let Err(e) = self.draft.set(info.name, new) {
                                    self.error = Some(e);
                                }
                            }
                            ui.end_row();
                        }
                    });
                    if group == "Saved results" {
                        ui.add_space(8.0);
                        match &app.store {
                            Some(st) => {
                                let s = st.stats();
                                ui.label(format!(
                                    "{} saved for {} in {} ({} MB, of which graphs {} MB).",
                                    plural(s.results as usize, "result"),
                                    plural(s.files as usize, "file"),
                                    s.path.display(),
                                    f(s.disk_bytes as f64 / 1e6, 1),
                                    f(s.graph_bytes as f64 / 1e6, 1)
                                ));
                            }
                            None => {
                                ui.label("Saving results is turned off, or the saved results cannot be opened.");
                            }
                        }
                        if ui.button("Delete all saved results…").clicked() {
                            app.forget_saved_ask();
                        }
                    }
                });
                if let Some(e) = &self.error {
                    ui.label(RichText::new(e).color(hex(0xc62828)));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Restore defaults").clicked() {
                        self.draft = Settings::default();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            result = Some(false);
                        }
                        if ui.button(RichText::new("Save").strong()).clicked() {
                            result = Some(true);
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
            ui.label(RichText::new(&self.head).strong().size(15.0));
            ui.label(RichText::new(
                "Each file is rewritten at the sample rate and bit depth it really contains, so its properties show the actual values (e.g. a fake 24/96 upsampled from a CD becomes a genuine 16/44.1 file). It keeps its name, tags and cover art, nothing audible is lost and it gets smaller. (Only editing the numbers in the file would make it play at the wrong speed, so the audio itself is converted.)",
            ).color(hex(0x555555)));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(220.0).id_salt("fixrows").show(ui, |ui| {
                egui::Grid::new("fixgrid").num_columns(4).striped(true).spacing([14.0, 3.0]).show(ui, |ui| {
                    for h in ["File", "Claimed → real", "Status", "Result"] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for r in &self.rows {
                        let color = match r.kind {
                            RowKind::Skip => hex(0x888888),
                            RowKind::Ok => hex(0x2e7d32),
                            RowKind::Fail => hex(0xc62828),
                            RowKind::Ready => ui.visuals().text_color(),
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
            ui.add_space(6.0);
            let editable = !self.started;
            ui.add_enabled_ui(editable, |ui| {
                egui::Grid::new("fixopts").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
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
                            if ui.button("Browse…").clicked() {
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
                            if ui.button("Browse…").clicked() {
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
                    ui.label(RichText::new("Empty backup folder = a 'Real-Hi-Res backups' folder next to each file (never scanned).").small().color(hex(0x777777)));
                    ui.end_row();
                });
            });
            if !self.note.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.note).color(hex(0x9a6700)));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let n = self.plans.len().max(1);
                ui.add(egui::ProgressBar::new(self.done as f32 / n as f32).desired_width(200.0));
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                    if ui.add_enabled(!self.plans.is_empty() && !self.started, egui::Button::new(RichText::new("Fix files").strong())).clicked() {
                        self.ask_start(app);
                    }
                    if ui.add_enabled(!self.done_paths.is_empty(), egui::Button::new("Show in folder")).clicked() {
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
                    text: "Conversions are still running. Close anyway? Files being converted are finished in the background.".into(),
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
