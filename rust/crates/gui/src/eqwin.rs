//! The Equalizer and Playback info windows of the player (2.0).

use egui::{pos2, vec2, Align2, Color32, CornerRadius, Rect, RichText, Sense, Shape, Stroke};
use realhires::eq::{self, Band, BAND_TYPES, GRAPHIC_LABELS, MAX_PARAMETRIC_BANDS, PRESETS};
use realhires::musiclib::Track;
use realhires::playerstate::PlayerState;

use crate::audio::OutputInfo;
use crate::look::{self, Palette};
use crate::player::{gain_of, ConfirmAction, NameAction, Player};

/// "44.1 kHz", "96 kHz".
pub fn khz(rate: i64) -> String {
    if rate <= 0 {
        return "—".into();
    }
    let k = rate as f64 / 1000.0;
    if (k - k.round()).abs() < 1e-9 {
        format!("{} kHz", k.round() as i64)
    } else {
        format!("{} kHz", realhires::pyfmt::g(k))
    }
}

fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn db(v: f64) -> String {
    let t = format!("{:.1} dB", v.abs());
    if v < -0.05 {
        format!("−{t}")
    } else if v > 0.05 {
        format!("+{t}")
    } else {
        "0.0 dB".into()
    }
}

fn channels(n: i64) -> String {
    match n {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        0 => "—".into(),
        n => format!("{n} channels"),
    }
}

/// "Off", "Graphic: Rock", "Parametric: 3 bands", with the pre-amp.
pub fn eq_summary(s: &PlayerState) -> String {
    let e = &s.eq;
    if !e.enabled {
        return "Off".into();
    }
    let mut parts = vec![];
    if e.graphic_enabled {
        parts.push(format!("Graphic: {}", e.preset));
    }
    if e.parametric_enabled {
        let n = e.bands.iter().filter(|b| b.on).count();
        parts.push(format!("Parametric: {}", crate::style::plural(n, "band")));
    }
    if parts.is_empty() {
        return "On, no bands".into();
    }
    let pre = eq::preamp_db(e, eq::RESPONSE_RATE);
    let mut text = parts.join(" + ");
    if pre.abs() >= 0.05 {
        text += &format!(" (pre-amp {})", db(pre));
    }
    text
}

/// The sections of the Playback info window.
pub fn info_sections(
    t: &Track,
    album_tracks: usize,
    out: Option<&OutputInfo>,
    state: &PlayerState,
    gapless: bool,
    verdict: Option<&(String, String)>,
) -> Vec<(&'static str, Vec<(&'static str, String)>)> {
    let dash = |s: &str| if s.trim().is_empty() { "—".to_string() } else { s.to_string() };
    let track_no = match (t.track, album_tracks) {
        (0, _) => "—".to_string(),
        (n, total) if total > 1 => format!("{n} of {total}"),
        (n, _) => n.to_string(),
    };
    let track = vec![
        ("Title", t.display_title()),
        ("Artist", dash(&t.artist)),
        ("Album", dash(&t.album)),
        ("Album artist", dash(&t.album_artist)),
        ("Genre", dash(&t.genre)),
        ("Year", dash(&t.year)),
        ("Track", track_no),
        ("Disc", if t.disc > 0 { t.disc.to_string() } else { "—".into() }),
        ("Duration", crate::player::fmt_time(t.duration)),
        ("File", t.path.clone()),
        ("Size", if t.size > 0 { realhires::fix::fmt_bytes(t.size as f64) } else { "—".into() }),
    ];
    let bits = if t.is_lossy {
        "—".to_string()
    } else if t.is_float {
        "32-bit float".into()
    } else if t.bits > 0 {
        format!("{}-bit", t.bits)
    } else {
        "—".into()
    };
    let version = crate::audio::ffmpeg_version();
    let decoder = vec![
        (
            "Format",
            if t.codec.is_empty() {
                "—".into()
            } else {
                format!("{} ({})", t.codec.to_uppercase(), if t.is_lossy { "lossy" } else { "lossless" })
            },
        ),
        ("Sample rate", khz(t.sample_rate)),
        ("Bit depth", bits),
        ("Channels", channels(t.channels)),
        ("Bit rate", if t.bit_rate > 0 { format!("{} kbps", thousands((t.bit_rate as f64 / 1000.0).round() as i64)) } else { "—".into() }),
        (
            "Decoder",
            match version.split(" Copyright").next().map(str::trim).filter(|v| !v.is_empty()) {
                Some(v) => format!("FFmpeg ({v})"),
                None => "FFmpeg".into(),
            },
        ),
    ];
    let gain = gain_of(state);
    let output = match out {
        Some(o) => vec![
            ("Device", o.device.clone()),
            ("Output", format!("{}, {}, {}", khz(o.rate as i64), o.format, channels(o.channels as i64).to_lowercase())),
            (
                "Resampling",
                match o.resampled_from {
                    None => "None: the device runs at the song's rate".into(),
                    Some(from) => format!(
                        "From {} to {} ({})",
                        khz(from as i64),
                        khz(o.rate as i64),
                        if o.soxr { "soxr" } else { "FFmpeg's resampler" }
                    ),
                },
            ),
            ("Volume", if gain <= 0.0 { "Muted".into() } else { db(20.0 * (gain as f64).log10()) }),
            ("Equalizer", eq_summary(state)),
            ("Gapless", if gapless { "On".into() } else { "Off".into() }),
        ],
        None => vec![("Device", "Not open yet".into())],
    };
    let check = vec![match verdict {
        Some((v, s)) => ("Verdict", format!("{v}: {s}")),
        None => ("Verdict", "Not checked yet".into()),
    }];
    vec![("Track", track), ("Decoder", decoder), ("Output", output), ("Quality check", check)]
}

pub fn info_window(p: &mut Player, ctx: &egui::Context, pal: &Palette) {
    let Some(info) = &p.info else { return };
    let path = info.path.clone();
    let t = p.lib.track(&path).cloned().unwrap_or_else(|| Track { path: path.clone(), ..Default::default() });
    let n_album = p.lib.album_of(&path).map(|a| a.tracks.len()).unwrap_or(0);
    let out = p.engine.info();
    let sections = info_sections(&t, n_album, out.as_ref(), &p.state, p.scanner.cfg.gapless, info.verdict.as_ref());
    let mut open = true;
    egui::Window::new("Playback info")
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(860.0)
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            let section = |ui: &mut egui::Ui, title: &str, rows: &[(&'static str, String)]| {
                ui.label(RichText::new(title).font(look::title(15.0)).color(pal.text));
                ui.add_space(2.0);
                egui::Grid::new(("info", title)).num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(RichText::new(*k).color(pal.secondary));
                        ui.add(egui::Label::new(RichText::new(v).color(pal.text)).wrap());
                        ui.end_row();
                    }
                });
                ui.add_space(12.0);
            };
            egui::ScrollArea::vertical().max_height(560.0).show(ui, |ui| {
                ui.columns(2, |cols| {
                    if let Some((t, rows)) = sections.first() {
                        section(&mut cols[0], t, rows);
                    }
                    for (t, rows) in sections.iter().skip(1) {
                        section(&mut cols[1], t, rows);
                    }
                });
            });
            ui.separator();
            if ui.button("Copy").clicked() {
                let text: String = sections
                    .iter()
                    .map(|(t, rows)| {
                        format!("{t}\n{}", rows.iter().map(|(k, v)| format!("  {k}: {v}")).collect::<Vec<_>>().join("\n"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n");
                ui.ctx().copy_text(text);
            }
        });
    if !open {
        p.info = None;
    }
}

/// The combined response curve, 20 Hz to 20 kHz, ±12 dB.
fn response_plot(ui: &mut egui::Ui, pal: &Palette, state: &eq::EqState) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 130.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(8), pal.lcd);
    let x_of = |f: f64| rect.left() + rect.width() * ((f / 20.0).log10() / 3.0) as f32;
    let y_of = |d: f64| rect.center().y - (d.clamp(-14.0, 14.0) / 14.0) as f32 * (rect.height() / 2.0 - 8.0);
    for d in [-12.0, -6.0, 0.0, 6.0, 12.0] {
        let y = y_of(d);
        p.line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(if d == 0.0 { 1.0 } else { 0.5 }, pal.separator));
        p.text(pos2(rect.left() + 4.0, y - 1.0), Align2::LEFT_BOTTOM, format!("{d:+}"), look::text(9.0), pal.secondary);
    }
    for (f, label) in [(32.0, "32"), (125.0, "125"), (500.0, "500"), (2000.0, "2K"), (8000.0, "8K"), (16000.0, "16K")] {
        let x = x_of(f);
        p.line_segment([pos2(x, rect.top()), pos2(x, rect.bottom())], Stroke::new(0.5, pal.separator));
        p.text(pos2(x + 2.0, rect.bottom() - 2.0), Align2::LEFT_BOTTOM, label, look::text(9.0), pal.secondary);
    }
    let pts: Vec<egui::Pos2> = eq::response(state, eq::RESPONSE_RATE).into_iter().map(|(f, d)| pos2(x_of(f), y_of(d))).collect();
    let color = if state.enabled { pal.accent } else { pal.secondary };
    p.add(Shape::line(pts, Stroke::new(2.0, color)));
}

enum EqAction {
    SaveAs,
    Rename(String),
    Delete(String),
    Import,
    Export(String),
}

pub fn eq_window(p: &mut Player, ctx: &egui::Context, pal: &Palette) {
    let mut open = true;
    let mut state = p.state.eq.clone();
    let mut tab = p.eq_tab;
    let mine = p.state.matching_eq_preset();
    let presets: Vec<(String, eq::EqState)> = p.state.eq_presets.iter().map(|x| (x.name.clone(), x.eq.clone())).collect();
    let notes = p.eq_notes.clone();
    let mut action: Option<EqAction> = None;
    egui::Window::new("Equalizer")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .fixed_size([660.0, 520.0])
        .pivot(Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            // presets: the built-in ones (graphic gains) and the user's own (the whole EQ)
            ui.horizontal(|ui| {
                ui.label("Preset");
                let shown = if !mine.is_empty() { mine.clone() } else { state.preset.clone() };
                egui::ComboBox::from_id_salt("eq-all-presets").selected_text(shown).width(170.0).height(440.0).show_ui(ui, |ui| {
                    ui.label(RichText::new("Built-in").small().color(pal.secondary));
                    for (name, _) in PRESETS.iter() {
                        if ui.selectable_label(mine.is_empty() && state.preset == *name, *name).clicked() {
                            state.apply_preset(name);
                            state.graphic_enabled = true;
                            state.enabled = true;
                        }
                    }
                    if !presets.is_empty() {
                        ui.separator();
                        ui.label(RichText::new("My presets").small().color(pal.secondary));
                        for (name, st) in &presets {
                            if ui.selectable_label(mine == *name, name).clicked() {
                                state = st.clone();
                            }
                        }
                    }
                });
                if ui.button("Save as preset…").clicked() {
                    action = Some(EqAction::SaveAs);
                }
                ui.add_enabled_ui(!mine.is_empty(), |ui| {
                    if ui.button("Rename…").clicked() {
                        action = Some(EqAction::Rename(mine.clone()));
                    }
                    if ui.button("Delete…").clicked() {
                        action = Some(EqAction::Delete(mine.clone()));
                    }
                });
                if ui.button("Import…").on_hover_text("Real-Hi-Res presets (.json) and Equalizer APO or AutoEQ files (.txt)").clicked() {
                    action = Some(EqAction::Import);
                }
                if ui.button("Export…").on_hover_text("As a Real-Hi-Res preset (.json) or as Equalizer APO / AutoEQ text (.txt)").clicked() {
                    action = Some(EqAction::Export(if mine.is_empty() { state.preset.clone() } else { mine.clone() }));
                }
            });
            for n in &notes {
                ui.label(RichText::new(n).small().color(pal.secondary));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut state.enabled, RichText::new("Equalizer on").strong());
                ui.add_space(12.0);
                ui.checkbox(&mut state.prevent_clipping, "Prevent clipping").on_hover_text(
                    "Lowers the level by as much as the equalizer raises it at its highest point, so loud passages never clip.",
                );
            });
            ui.add_space(6.0);
            response_plot(ui, pal, &state);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                for (i, name) in ["Graphic", "Parametric"].iter().enumerate() {
                    if ui.add(egui::Button::selectable(tab == i, *name)).clicked() {
                        tab = i;
                    }
                }
            });
            ui.separator();
            ui.add_enabled_ui(state.enabled, |ui| {
                if tab == 0 {
                    graphic(ui, pal, &mut state);
                } else {
                    parametric(ui, pal, &mut state);
                }
            });
        });
    p.eq_tab = tab;
    match action {
        Some(EqAction::SaveAs) => p.ask_name("Save Equalizer Preset", "My Preset", NameAction::SaveEqPreset),
        Some(EqAction::Rename(n)) => p.ask_name("Rename Preset", &n.clone(), NameAction::RenameEqPreset(n)),
        Some(EqAction::Delete(n)) => p.confirm = Some(ConfirmAction::DeleteEqPreset(n)),
        Some(EqAction::Import) => {
            if let Some(path) = rfd::FileDialog::new().add_filter("Equalizer presets", &["json", "txt"]).pick_file() {
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into());
                let result = std::fs::read(&path)
                    .map_err(|e| format!("The file could not be read: {e}."))
                    .and_then(|b| p.import_eq_text(&String::from_utf8_lossy(&b), &stem));
                match result {
                    Ok(name) => p.eq_notes.insert(0, format!("Imported as “{name}”.")),
                    Err(e) => p.eq_notes = vec![e],
                }
                state = p.state.eq.clone();
            }
        }
        Some(EqAction::Export(name)) => {
            let file = rfd::FileDialog::new()
                .add_filter("Real-Hi-Res preset", &["json"])
                .add_filter("Equalizer APO / AutoEQ", &["txt"])
                .set_file_name(format!("{name}.json"))
                .save_file();
            if let Some(path) = file {
                let apo = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("txt"));
                let text = if apo { eq::export_apo(&name, &state) } else { eq::export_json(&name, &state) };
                p.eq_notes = vec![match std::fs::write(&path, text) {
                    Ok(()) => format!("Exported to {}.", path.display()),
                    Err(e) => format!("The preset could not be saved: {e}."),
                }];
            }
        }
        None => {}
    }
    if state != p.state.eq {
        p.state.eq = state;
        p.set_eq();
    }
    if !open {
        p.eq_open = false;
        p.save_state();
    }
}

fn graphic(ui: &mut egui::Ui, _pal: &Palette, s: &mut eq::EqState) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut s.graphic_enabled, "Use the graphic equalizer");
    });
    ui.add_space(6.0);
    ui.add_enabled_ui(s.graphic_enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label("Pre-amp");
            ui.add(egui::Slider::new(&mut s.graphic_preamp, -12.0..=12.0).suffix(" dB").step_by(0.5));
        });
        ui.add_space(8.0);
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.style_mut().visuals.slider_trailing_fill = false;
            for (i, label) in GRAPHIC_LABELS.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(48.0);
                    let r = ui.add(egui::Slider::new(&mut s.gains[i], -12.0..=12.0).vertical().step_by(0.5).show_value(false));
                    if r.changed() {
                        changed = true;
                    }
                    if r.double_clicked() {
                        s.gains[i] = 0.0;
                        changed = true;
                    }
                    r.on_hover_text(format!("{} Hz: {} (double-click resets it)", label, db(s.gains[i])));
                    ui.label(RichText::new(*label).small());
                    ui.label(RichText::new(format!("{:+.1}", s.gains[i])).small().weak());
                });
            }
        });
        if changed {
            s.preset = eq::preset_name(&s.gains);
        }
    });
}

fn parametric(ui: &mut egui::Ui, pal: &Palette, s: &mut eq::EqState) {
    ui.checkbox(&mut s.parametric_enabled, "Use the parametric equalizer");
    ui.add_space(6.0);
    ui.add_enabled_ui(s.parametric_enabled, |ui| {
        ui.horizontal(|ui| {
            ui.label("Pre-amp");
            ui.add(egui::Slider::new(&mut s.parametric_preamp, -12.0..=12.0).suffix(" dB").step_by(0.5));
        });
        ui.add_space(6.0);
        let mut remove = None;
        egui::ScrollArea::vertical().max_height(250.0).show(ui, |ui| egui::Grid::new("eq-bands").num_columns(7).spacing([10.0, 6.0]).show(ui, |ui| {
            for h in ["On", "Type", "Frequency", "Gain", "Q", ""] {
                ui.label(RichText::new(h).small().color(pal.secondary));
            }
            ui.end_row();
            for (i, b) in s.bands.iter_mut().enumerate() {
                ui.checkbox(&mut b.on, "");
                egui::ComboBox::from_id_salt(("band-type", i)).selected_text(eq::band_type_label(&b.kind)).width(110.0).show_ui(ui, |ui| {
                    for k in BAND_TYPES {
                        ui.selectable_value(&mut b.kind, k.to_string(), eq::band_type_label(k));
                    }
                });
                let speed = b.freq * 0.01;
                ui.add(egui::DragValue::new(&mut b.freq).range(eq::MIN_FREQ..=eq::MAX_FREQ).speed(speed).suffix(" Hz").max_decimals(0));
                let has_gain = eq::has_gain(&b.kind);
                ui.add_enabled(has_gain, egui::DragValue::new(&mut b.gain).range(-12.0..=12.0).speed(0.1).suffix(" dB").max_decimals(1));
                ui.add(egui::DragValue::new(&mut b.q).range(eq::MIN_Q..=eq::MAX_Q).speed(0.01).max_decimals(2));
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        }));
        if let Some(i) = remove {
            s.bands.remove(i);
        }
        ui.add_space(6.0);
        if ui.add_enabled(s.bands.len() < MAX_PARAMETRIC_BANDS, egui::Button::new("Add band")).clicked() {
            s.bands.push(Band::default());
        }
    });
    let _ = Color32::WHITE;
}
