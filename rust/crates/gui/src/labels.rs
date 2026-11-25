//! Colour labels (2.3, `docs/player-spec.md`, "9. Colours"): "Colour Label ▸" in the
//! menus of songs, albums, artists, album artists, genres, years, composers,
//! playlists (and tabs), with Red … Grey, Custom… (a colour picker in a small
//! window) and None; the label is kept in `player.json` (`PlayerState::label_of()`,
//! `set_label()`) and drawn as a dot 8 px wide (`paint_dot()`): before the title in
//! song tables and Up Next, at the bottom-left corner of a card's cover, and after a
//! playlist's name in the sidebar.  The counterpart of the Python app's
//! `PlayerApp.add_label_menu()` and `player_widgets.draw_label_dot()`.

use egui::{pos2, vec2, Color32, Painter, Pos2, Rect, RichText, Sense, Ui};
use realhires::i18n;
use realhires::playerstate::LABEL_COLORS;
use realhires::tr;

use crate::look::{self, Palette};
use crate::player::Player;

/// The width of a label's dot, in pixels.
pub const DOT: f32 = 8.0;

/// A colour label's dot centred on `center`; with `ring`, inside a 2-pixel ring of that
/// colour (on a cover, where the dot needs to stand out).
pub fn paint_dot(p: &Painter, center: Pos2, color: Color32, ring: Option<Color32>) {
    if let Some(r) = ring {
        p.circle_filled(center, DOT / 2.0 + 2.0, r);
    }
    p.circle_filled(center, DOT / 2.0, color);
}

/// The dot at the bottom-left corner of a card's cover `cover`.
pub fn paint_cover_dot(p: &Painter, cover: Rect, color: Color32, pal: &Palette) {
    paint_dot(p, cover.left_bottom() + vec2(14.0, -14.0), color, Some(pal.bg));
}

/// The window of Colour Label → Custom… (one at a time, `Player::label_picker`).
#[derive(Clone, Debug)]
pub struct LabelPicker {
    pub kind: String,
    pub key: String,
    pub color: Color32,
}

/// One entry of the Colour Label menu: a swatch, the colour's name, and the one in
/// use marked.
fn swatch_entry(ui: &mut Ui, color: Color32, label: &str, on: bool) -> egui::Response {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 5.5, color);
        ui.add(egui::Button::new(label).selected(on).frame(on))
    })
    .inner
}

impl Player {
    /// The colour label of a song (path), album (key), artist, album artist, genre, year
    /// or composer (name), playlist (id) or tab (name), or None.
    pub fn label_of(&self, kind: &str, key: &str) -> Option<Color32> {
        self.state.label_of(kind, key).map(look::parse_hex)
    }

    /// Give it a colour label ("#rrggbb"; None takes it away), saved at once.
    pub fn set_label(&mut self, kind: &str, key: &str, color: Option<&str>) {
        self.state.set_label(kind, key, color);
        self.save_state();
    }

    /// "Colour Label ▸" (2.3) in a menu: Red, Orange, Yellow, Green, Blue, Purple, Grey,
    /// Custom… (opens the colour picker window) and None, the label in use marked.
    /// `kind` is one of `playerstate::LABEL_KINDS` ("song", "album", "artist",
    /// "album_artist", "genre", "year", "composer", "playlist", "tab") and `key` what it is
    /// kept under (a path, an album key, a name, a playlist id, a tab name).
    pub fn label_menu(&mut self, ui: &mut Ui, kind: &str, key: &str) {
        ui.menu_button(tr!("Colour Label"), |ui| {
            let current = self.state.label_of(kind, key).map(str::to_string);
            for (name, hex) in LABEL_COLORS {
                if swatch_entry(ui, look::parse_hex(hex), i18n::tr(name), current.as_deref() == Some(hex)).clicked() {
                    self.set_label(kind, key, Some(hex));
                    ui.close();
                }
            }
            ui.separator();
            let custom = current.as_deref().is_some_and(|c| !LABEL_COLORS.iter().any(|(_, h)| *h == c));
            if ui.add(egui::Button::new(tr!("Custom…")).selected(custom).frame(custom)).clicked() {
                let color = current.as_deref().map(look::parse_hex).unwrap_or_else(|| look::parse_hex("#8e8e93"));
                self.label_picker = Some(LabelPicker { kind: kind.to_string(), key: key.to_string(), color });
                ui.close();
            }
            if ui.add_enabled(current.is_some(), egui::Button::new(tr!("None"))).clicked() {
                self.set_label(kind, key, None);
                ui.close();
            }
        });
    }

    /// The Custom… window: a colour picker with OK and Cancel.  Returns whether it is open.
    pub fn label_picker_window(&mut self, ctx: &egui::Context, pal: &Palette) -> bool {
        let Some(mut picker) = self.label_picker.take() else { return false };
        let mut open = true;
        let mut done: Option<bool> = None;
        egui::Window::new(tr!("Colour Label"))
            .id(egui::Id::new("label-picker"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(RichText::new(tr!("Choose the colour of the label.")).color(pal.secondary));
                ui.add_space(6.0);
                egui::color_picker::color_picker_color32(ui, &mut picker.color, egui::color_picker::Alpha::Opaque);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    paint_dot(ui.painter(), r.center(), picker.color, None);
                    ui.label(RichText::new(look::to_hex(picker.color)).color(pal.secondary));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if crate::style::primary_button(ui, tr!("OK"), true).clicked() {
                            done = Some(true);
                        }
                        if crate::style::flat_button(ui, tr!("Cancel"), true).clicked() {
                            done = Some(false);
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            done = Some(false);
        }
        match done {
            Some(true) => {
                let hex = look::to_hex(picker.color);
                self.set_label(&picker.kind, &picker.key, Some(&hex));
                false
            }
            Some(false) => false,
            None if !open => false,
            None => {
                self.label_picker = Some(picker);
                true
            }
        }
    }

    /// The menu of a playlist (2.3: right click in the sidebar and on the Playlists page):
    /// Play, Shuffle, Rename…, Delete… and Colour Label.
    pub fn playlist_menu(&mut self, ui: &mut Ui, id: &str) {
        let Some(p) = self.state.playlist(id) else {
            ui.close();
            return;
        };
        let (name, songs) = (p.name.clone(), self.playlist_songs(id));
        let songs: Vec<String> = songs.into_iter().filter(|s| self.lib.track(s).is_some()).collect();
        if ui.add_enabled(!songs.is_empty(), egui::Button::new(tr!("Play"))).clicked() {
            self.play_list(songs.clone(), Some(&songs[0]), &name);
            ui.close();
        }
        if ui.add_enabled(!songs.is_empty(), egui::Button::new(tr!("Shuffle"))).clicked() {
            self.shuffle_list(songs, &name);
            ui.close();
        }
        ui.separator();
        if ui.button(tr!("Rename…")).clicked() {
            self.ask_name(tr!("Rename Playlist"), &name, crate::player::NameAction::RenamePlaylist(id.to_string()));
            ui.close();
        }
        if ui.button(tr!("Delete…")).clicked() {
            self.confirm = Some(crate::player::ConfirmAction::DeletePlaylist(id.to_string()));
            ui.close();
        }
        ui.separator();
        self.label_menu(ui, "playlist", id);
    }
}

/// The dot of a sidebar entry's label, after its name drawn from `text_left` with
/// `width` pixels of text.
pub fn paint_after_text(p: &Painter, text_left: f32, width: f32, y: f32, color: Color32) {
    paint_dot(p, pos2(text_left + width + 6.0 + DOT / 2.0, y), color, None);
}
