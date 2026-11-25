//! Now Playing's cover as a record sleeve (2.5, `docs/player-spec.md`, "2.5", items 2
//! and 9), the counterpart of Python's `player_jacket.py`.
//!
//! The square of the cover shows the cover, or its back: the album's songs over the
//! cover mirrored and blurred (`covers::back_picture()`, made once per cover in a
//! thread), never larger than the square.  A swipe, the dots, a sideways scroll or the
//! arrow keys turn the jacket about its vertical axis (`realhires::jacket::turn_frame()`):
//! the face is painted as a textured mesh whose far edge is shorter, so the turn has its
//! perspective, and darkened while it is turned away.  A click on the cover opens the
//! album; a right click opens the cover's menu (Cover…, Change Cover…, Search Cover
//! Online ▸, Go to Album).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Instant;

use egui::{pos2, vec2, Color32, CornerRadius, Pos2, Rect, Sense, Ui};
use realhires::i18n::tr_name;
use realhires::jacket::{self as core, TurnFrame};
use realhires::musiclib::Album;
use realhires::{covers, tr};

use crate::cover_cache::LARGE_PX;
use crate::icons::{self, Icon};
use crate::look::{self, Palette};
use crate::pages::elided;
use crate::player::{fmt_time, Page, Player};

/// A press that moves further is a drag, not a click.
const DRAG_START: f32 = 5.0;
/// The cover's rounded corners on Now Playing.
pub const RADIUS: u8 = 12;
/// Backs kept (each a 480 x 480 texture).
const MAX_KEPT: usize = 12;

struct Turn {
    /// the face it started from is the back (the songs)
    from_songs: bool,
    dir: i32,
    p: f64,
    /// an eased move: (from, to, since, milliseconds); None while the pointer drags it
    anim: Option<(f64, f64, Instant, i64)>,
    drag: bool,
}

struct Back {
    tex: egui::TextureHandle,
    /// the cover's texture it was made from (a changed cover gets a new back)
    source: egui::TextureId,
}

type Made = (String, egui::TextureId, Option<egui::ColorImage>);

/// What a frame of the jacket asks of the page.
#[derive(Debug, PartialEq)]
pub enum Action {
    Nothing,
    /// a click on the cover: the album's page
    OpenAlbum,
}

pub struct Jacket {
    turn: Option<Turn>,
    /// since when the back's text is fading in
    fade: Option<Instant>,
    backs: HashMap<String, Back>,
    order: VecDeque<String>,
    pending: HashSet<String>,
    tx: Sender<Made>,
    rx: Receiver<Made>,
    /// frames of turns painted (the tests count them)
    pub frames: usize,
    pub last_frame: Option<TurnFrame>,
    /// what the square holds this frame: "cover", "songs" or "turn"
    pub shows: &'static str,
    /// the bounds of everything painted for the square this frame (never outside it)
    pub painted: Option<Rect>,
    /// the back's thin scroll bar, when more songs than fit
    pub bar: Option<Rect>,
    /// how far the back's songs are scrolled, and how tall they are
    pub scroll: (f32, f32),
}

impl Default for Jacket {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Jacket {
            turn: None,
            fade: None,
            backs: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            tx,
            rx,
            frames: 0,
            last_frame: None,
            shows: "cover",
            painted: None,
            bar: None,
            scroll: (0.0, 0.0),
        }
    }
}

impl Jacket {
    /// A turn (or the fade after it) is going on.
    pub fn turning(&self) -> bool {
        self.turn.is_some() || self.fade.is_some()
    }

    /// The back of this album's cover is made.
    pub fn ready(&self, key: &str) -> bool {
        self.backs.contains_key(key)
    }

    /// How far the turn going on has come (0..1), for the tests.
    pub fn progress(&self) -> Option<f64> {
        self.turn.as_ref().map(|t| t.p)
    }

    fn fade_alpha(&self) -> f32 {
        match self.fade {
            Some(t) if core::fade_ms() > 0 => (t.elapsed().as_secs_f32() * 1000.0 / core::fade_ms() as f32).min(1.0),
            _ => 1.0,
        }
    }
}

/// The back of a cover, from its picture in the covers folder: mirrored, blurred and
/// under the scrim (`covers::back_picture()`).
fn make_back(paths: &[String]) -> Option<egui::ColorImage> {
    let png = covers::cached_cover_png(paths, LARGE_PX)?;
    let img = image::load_from_memory(&png).ok()?.to_rgb8();
    let (w, h) = (img.width() as usize, img.height() as usize);
    let back = covers::back_picture(img.as_raw(), w, h, 0, covers::BACK_SCRIM);
    (back.len() == w * w * 3).then(|| egui::ColorImage::from_rgb([w, w], &back))
}

/// The points of a square with rounded corners, in the square's own measure (0..1),
/// clockwise from the top left.
fn outline(radius: f32) -> Vec<Pos2> {
    let r = radius.clamp(0.0, 0.5);
    let mut pts = vec![];
    // the centre of each corner's circle, and the angle its arc starts at (quarter turns)
    for (cx, cy, start) in [(r, r, 2), (1.0 - r, r, 3), (1.0 - r, 1.0 - r, 0), (r, 1.0 - r, 1)] {
        for k in 0..=6 {
            let a = (start as f32 + k as f32 / 6.0) * std::f32::consts::FRAC_PI_2;
            pts.push(pos2(cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    pts
}

/// The colour with `keep` of its light (0 = black), as opaque as it was.
pub fn darker(c: Color32, keep: f32) -> Color32 {
    let k = |v: u8| (v as f32 * keep.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(k(c.r()), k(c.g()), k(c.b()))
}

/// One face of the jacket in the middle of a turn: `frame.width` wide about the middle of
/// `rect`, its far edge `frame.far` of the height, darkened; a fan of triangles from the
/// middle to the rounded outline, textured (or of one colour).
pub fn turn_mesh(rect: Rect, frame: &TurnFrame, tex: Option<egui::TextureId>, flat: Color32) -> egui::Mesh {
    let keep = (1.0 - frame.dark).clamp(0.0, 1.0) as f32;
    let color = match tex {
        Some(_) => Color32::from_gray((keep * 255.0).round() as u8),
        None => darker(flat, keep),
    };
    let mut mesh = match tex {
        Some(id) => egui::Mesh::with_texture(id),
        None => egui::Mesh::default(),
    };
    let width = (frame.width as f32).max(2.0 / rect.width().max(1.0)); // edge-on: the sleeve's own thickness
    let place = |u: f32, v: f32| -> Pos2 {
        let away = if frame.far_right { u } else { 1.0 - u };
        let height = 1.0 - (1.0 - frame.far as f32) * away;
        pos2(rect.center().x + (u - 0.5) * rect.width() * width, rect.center().y + (v - 0.5) * rect.height() * height)
    };
    let uv = |u: f32, v: f32| if tex.is_some() { pos2(u, v) } else { egui::epaint::WHITE_UV };
    mesh.vertices.push(egui::epaint::Vertex { pos: place(0.5, 0.5), uv: uv(0.5, 0.5), color });
    let pts = outline(RADIUS as f32 / rect.width().max(1.0));
    for p in &pts {
        mesh.vertices.push(egui::epaint::Vertex { pos: place(p.x, p.y), uv: uv(p.x, p.y), color });
    }
    let n = pts.len() as u32;
    for k in 0..n {
        mesh.indices.extend([0, 1 + k, 1 + (k + 1) % n]);
    }
    mesh
}

impl Player {
    /// Turn Now Playing's jacket to the cover (false) or the album's songs (true): the
    /// dots, a sideways scroll, the keys.  `dir`: which way the pointer went (0: to the
    /// songs as a drag to the left).
    pub fn turn_jacket(&mut self, songs: bool, dir: i32) {
        if songs == self.np_songs {
            return;
        }
        let dir = if dir != 0 {
            dir
        } else if songs {
            -1
        } else {
            1
        };
        let ms = core::flip_ms();
        let old = self.jacket.turn.take();
        self.np_songs = songs;
        self.jacket.fade = None;
        if ms <= 0 || self.page != Page::NowPlaying || self.now.is_none() {
            return;
        }
        // turned back in the middle of a turn: from where it is
        let (p, dir) = match old {
            Some(t) if t.from_songs == songs => (1.0 - t.p, -t.dir), // it was on its way to the other face
            Some(t) => (t.p, t.dir),                                  // a drag, or a turn springing back
            None => (0.0, dir),
        };
        self.jacket.turn = Some(Turn { from_songs: !songs, dir, p, anim: Some((p, 1.0, Instant::now(), core::rest_ms(p, true, ms))), drag: false });
    }

    /// Take the backs the threads made, and ask for the one of this cover if it is missing.
    fn jacket_backs(&mut self, ctx: &egui::Context, key: &str, paths: &[String], tex: Option<&egui::TextureHandle>) {
        while let Ok((k, source, image)) = self.jacket.rx.try_recv() {
            self.jacket.pending.remove(&k);
            let Some(image) = image else { continue };
            let tex = ctx.load_texture(format!("jacket back {k}"), image, egui::TextureOptions::LINEAR);
            self.jacket.order.retain(|o| *o != k);
            self.jacket.order.push_back(k.clone());
            self.jacket.backs.insert(k, Back { tex, source });
            while self.jacket.order.len() > MAX_KEPT {
                if let Some(old) = self.jacket.order.pop_front() {
                    self.jacket.backs.remove(&old);
                }
            }
        }
        let Some(tex) = tex else {
            self.jacket.backs.remove(key); // no cover (any more): a plain back
            return;
        };
        let fresh = self.jacket.backs.get(key).is_some_and(|b| b.source == tex.id());
        if !fresh && !self.jacket.pending.contains(key) {
            self.jacket.pending.insert(key.to_string());
            let (tx, k, paths, source, ctx) = (self.jacket.tx.clone(), key.to_string(), paths.to_vec(), tex.id(), ctx.clone());
            let _ = std::thread::Builder::new().name("jacket-back".into()).spawn(move || {
                let _ = tx.send((k, source, make_back(&paths)));
                ctx.request_repaint();
            });
        }
    }

    /// The square of Now Playing's cover (`rect`), in the block of `left`: the cover, its
    /// back or a frame of the turn, and what the pointer and the keys do to it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn np_jacket(&mut self, ui: &mut Ui, pal: &Palette, rect: Rect, left: Rect, tex: Option<&egui::TextureHandle>, path: &str, album: Option<&Album>, title: &str) -> Action {
        self.jacket.painted = None;
        self.jacket.bar = None;
        let Some(album) = album else {
            // a song without an album: the cover only
            self.jacket.turn = None;
            self.jacket.shows = "cover";
            crate::player::cover(ui.painter(), rect, tex, title, RADIUS);
            self.jacket.painted = Some(rect);
            let resp = ui.interact(rect, ui.scope_id().with("np-cover"), Sense::click());
            let path2 = path.to_string();
            resp.context_menu(|ui| self.cover_menu(ui, &path2, true));
            return Action::Nothing;
        };
        let paths: Vec<String> = album.tracks.iter().map(|t| t.path.clone()).collect();
        let ctx = ui.ctx().clone();
        self.jacket_backs(&ctx, &album.key, &paths, tex);

        // ---- the pointer: a sideways drag that started in the block turns the jacket with it
        let (origin, latest, down, hover, dx_scroll) =
            ui.input(|i| (i.pointer.press_origin(), i.pointer.latest_pos(), i.pointer.primary_down(), i.pointer.hover_pos(), i.smooth_scroll_delta.x));
        let over = hover.is_some_and(|h| rect.contains(h));
        let dragging = self.jacket.turn.as_ref().is_some_and(|t| t.drag);
        match (down, origin, latest) {
            (true, Some(o), Some(p)) if left.contains(o) => {
                let d = p - o;
                if !dragging && self.jacket.turn.is_none() && d.x.abs() > DRAG_START && d.x.abs() > d.y.abs() {
                    self.jacket.fade = None;
                    self.jacket.turn = Some(Turn { from_songs: self.np_songs, dir: 1, p: 0.0, anim: None, drag: true });
                }
                if let Some(t) = self.jacket.turn.as_mut().filter(|t| t.drag) {
                    t.dir = if d.x > 0.0 { 1 } else { -1 };
                    t.p = core::drag_progress(d.x as f64, rect.width() as f64);
                    self.np_swiped = core::drag_finishes(d.x as f64, rect.width() as f64); // (what letting go here would do)
                }
            }
            _ if dragging => {
                // let go: the turn finishes, or springs back
                let finish = self.np_swiped;
                self.np_swiped = false;
                let ms = core::flip_ms();
                if let Some(mut t) = self.jacket.turn.take() {
                    if finish {
                        self.np_songs = !t.from_songs;
                    }
                    if ms > 0 {
                        t.drag = false;
                        t.anim = Some((t.p, if finish { 1.0 } else { 0.0 }, Instant::now(), core::rest_ms(t.p, finish, ms)));
                        self.jacket.turn = Some(t);
                    }
                }
            }
            _ => {}
        }
        // a sideways scroll over the block: the content moving left shows the songs, to the right the cover
        if hover.is_some_and(|h| left.contains(h)) && dx_scroll != 0.0 {
            self.np_hscroll += dx_scroll;
            if self.np_hscroll.abs() > core::SCROLL_PX as f32 {
                let songs = self.np_hscroll < 0.0;
                self.turn_jacket(songs, if songs { -1 } else { 1 });
                self.np_hscroll = 0.0;
            }
        } else {
            self.np_hscroll = 0.0;
        }
        // ← / → while the pointer is over the jacket turn it (that way)
        if over && !down && !ctx.egui_wants_keyboard_input() {
            let key = ui.input(|i| {
                let plain = !i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.command;
                match (plain, i.key_pressed(egui::Key::ArrowLeft), i.key_pressed(egui::Key::ArrowRight)) {
                    (true, true, _) => -1,
                    (true, _, true) => 1,
                    _ => 0,
                }
            });
            if key != 0 {
                self.turn_jacket(!self.np_songs, key);
            }
        }

        // ---- the turn's own time
        if let Some(t) = self.jacket.turn.as_mut() {
            if let Some((p0, target, since, ms)) = t.anim {
                let elapsed = since.elapsed().as_secs_f64() * 1000.0;
                t.p = core::progress_at(p0, target, elapsed, ms as f64);
                if elapsed >= ms as f64 {
                    let ended_on_songs = if target >= 1.0 { !t.from_songs } else { t.from_songs };
                    self.jacket.turn = None;
                    if target >= 1.0 && ended_on_songs && core::fade_ms() > 0 {
                        self.jacket.fade = Some(Instant::now());
                    }
                }
            }
        }
        if self.jacket.fade.is_some() && self.jacket.fade_alpha() >= 1.0 {
            self.jacket.fade = None;
        }
        if self.jacket.turning() {
            ctx.request_repaint();
        }

        // ---- painting
        let back_tex = self.jacket.backs.get(&album.key).map(|b| b.tex.clone());
        let flat_back = darker(pal.placeholder, 1.0 - covers::BACK_SCRIM as f32);
        let mut action = Action::Nothing;
        if let Some(t) = &self.jacket.turn {
            let frame = core::turn_frame(t.p, t.dir);
            let songs_face = if frame.other { !t.from_songs } else { t.from_songs };
            let mesh = if songs_face {
                turn_mesh(rect, &frame, back_tex.as_ref().map(|b| b.id()), flat_back)
            } else {
                turn_mesh(rect, &frame, tex.map(|x| x.id()), pal.placeholder)
            };
            self.jacket.painted = Some(Rect::from_points(&mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>()));
            ui.painter().add(egui::Shape::mesh(mesh));
            self.jacket.frames += 1;
            self.jacket.last_frame = Some(frame);
            self.jacket.shows = "turn";
            // (the square still takes the pointer, so nothing under it is clicked through the turn)
            ui.interact(rect, ui.scope_id().with("np-cover"), Sense::click());
        } else if self.np_songs {
            self.jacket.shows = "songs";
            self.jacket.painted = Some(rect);
            match &back_tex {
                Some(b) => {
                    let shape = egui::epaint::RectShape::filled(rect, CornerRadius::same(RADIUS), Color32::WHITE)
                        .with_texture(b.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                    ui.painter().add(shape);
                }
                None => {
                    ui.painter().rect_filled(rect, CornerRadius::same(RADIUS), flat_back);
                }
            }
            let resp = ui.interact(rect, ui.scope_id().with("np-cover"), Sense::click());
            let path2 = path.to_string();
            resp.context_menu(|ui| self.cover_menu(ui, &path2, true));
            self.jacket_back(ui, rect, path, album);
        } else {
            self.jacket.shows = "cover";
            self.jacket.painted = Some(rect);
            crate::player::cover(ui.painter(), rect, tex, title, RADIUS);
            let resp = ui.interact(rect, ui.scope_id().with("np-cover"), Sense::click());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                action = Action::OpenAlbum; // a click without a drag (a drag is not a click)
            }
            let path2 = path.to_string();
            resp.context_menu(|ui| self.cover_menu(ui, &path2, true));
        }
        action
    }

    /// The back of the jacket: the album's title and artist, then its songs (number,
    /// title, time), inside the square `rect`, in white over the blurred cover.
    fn jacket_back(&mut self, ui: &mut Ui, rect: Rect, path: &str, album: &Album) {
        let l = core::back_layout(rect.width().round() as i64);
        let (pad, o) = (l.pad as f32, rect.min);
        let a = self.jacket.fade_alpha();
        let white = Color32::WHITE.gamma_multiply(a);
        let grey = Color32::WHITE.gamma_multiply(a * core::SECONDARY as f32);
        let painter = ui.painter_at(rect);
        let room = rect.width() - 2.0 * pad;
        let title = elided(ui, tr_name(&album.title), look::title(l.title_px as f32), white, room);
        painter.galley(pos2(o.x + pad, o.y + l.title_y as f32 - title.size().y / 2.0), title, white);
        let artist = elided(ui, tr_name(&album.artist), look::text(l.small_px as f32), grey, room);
        painter.galley(pos2(o.x + pad, o.y + l.artist_y as f32 - artist.size().y / 2.0), artist, grey);
        let inset = (pad / 2.0).max(3.0);
        let list = Rect::from_min_size(pos2(o.x + inset, o.y + l.list_top as f32), vec2(rect.width() - 2.0 * inset, (l.rows * l.row_h) as f32));
        let new_album = self.np_songs_album != album.key;
        self.np_songs_album = album.key.clone();
        let (text_font, bold_font) = (look::text(l.text_px as f32), look::title(l.text_px as f32));
        let mut play = None;
        let out = ui
            .scope_builder(egui::UiBuilder::new().max_rect(list), |ui| {
                ui.set_clip_rect(list.intersect(ui.clip_rect()));
                egui::ScrollArea::vertical()
                    .id_salt(("np-jacket-songs", &album.key))
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for t in &album.tracks {
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), l.row_h as f32), Sense::click());
                            let playing = t.path == path;
                            if playing && new_album {
                                resp.scroll_to_me(None);
                            }
                            if !ui.is_rect_visible(r) {
                                continue;
                            }
                            let p = ui.painter();
                            if resp.hovered() && a >= 1.0 {
                                let tint = Color32::from_white_alpha((core::HOVER_TINT * 255.0).round() as u8);
                                p.rect_filled(r.shrink2(vec2(0.0, 1.0)), CornerRadius::same(6), tint);
                            }
                            let mid = r.center().y;
                            let num_right = o.x + l.num_right as f32;
                            if playing {
                                let s = l.text_px as f32 + 2.0;
                                icons::paint(p, Rect::from_center_size(pos2(num_right - s * 0.5, mid), vec2(s, s)), Icon::Speaker, white);
                            } else if t.track > 0 {
                                p.text(pos2(num_right, mid), egui::Align2::RIGHT_CENTER, t.track.to_string(), text_font.clone(), grey);
                            }
                            let when = if t.duration > 0.0 { fmt_time(t.duration) } else { String::new() };
                            let time = p.text(pos2(o.x + l.time_right as f32, mid), egui::Align2::RIGHT_CENTER, &when, text_font.clone(), grey);
                            let x = o.x + l.title_x as f32;
                            let font = if playing { bold_font.clone() } else { text_font.clone() };
                            let name = elided(ui, &t.display_title(), font, white, time.left() - 10.0 - x);
                            p.galley(pos2(x, mid - name.size().y / 2.0), name, white);
                            if resp.double_clicked() {
                                play = Some(t.path.clone());
                            }
                            let path2 = path.to_string();
                            resp.context_menu(|ui| self.cover_menu(ui, &path2, true));
                            self.np_hits.album_songs.push((r.intersect(list), t.path.clone(), playing));
                        }
                    })
            })
            .inner;
        // more songs than fit: a thin bar
        let (view, content, offset) = (list.height(), out.content_size.y, out.state.offset().y);
        self.jacket.scroll = (offset, content);
        if content > view + 0.5 {
            let length = (view * view / content).max(12.0).min(view);
            let off = (view - length) * (offset / (content - view)).clamp(0.0, 1.0);
            let bar = Rect::from_min_size(pos2(o.x + (l.bar_right - l.bar_w) as f32, list.top() + off), vec2(l.bar_w as f32, length));
            painter.rect_filled(bar, CornerRadius::same(1), grey);
            self.jacket.bar = Some(bar);
        }
        if let Some(p) = play {
            let paths: Vec<String> = album.tracks.iter().map(|t| t.path.clone()).collect();
            self.play_list(paths, Some(&p), &album.title);
        }
    }

    /// Search Cover Online ▸ (2.5): MusicBrainz… (the cover search window) and Google
    /// Images (the browser), for the album of this song.
    pub(crate) fn search_cover_menu(&mut self, ui: &mut Ui, path: &str) {
        ui.menu_button(tr!("Search Cover Online"), |ui| {
            if ui.button(tr!("MusicBrainz…")).clicked() {
                self.open_cover_search(path);
                ui.close();
            }
            if ui.button(tr!("Google Images")).clicked() {
                self.search_cover_google(path);
                ui.close();
            }
        });
    }

    /// Search Cover Online → Google Images: the browser looks for the cover of the song's album.
    pub fn search_cover_google(&mut self, path: &str) {
        let url = match self.lib.track(path) {
            Some(t) => covers::cover_search_url(&t.album_artist, &t.artist, &t.album, &t.display_title()),
            None => covers::cover_search_url("", "", "", &realhires::paths::stem(path)),
        };
        let _ = crate::web::open_url(&url);
    }

    /// Search Cover Online → MusicBrainz…: the Cover window of the song's album, and in it
    /// the search for covers on MusicBrainz.
    pub fn open_cover_search(&mut self, path: &str) {
        self.open_cover_window(path);
        if let Some(w) = self.cover_win.as_mut().filter(|w| w.path == path) {
            w.open_search();
        }
    }

    /// The right-click menu of a cover (2.5): Cover… (the Cover window), Change Cover…,
    /// Search Cover Online ▸ and, on Now Playing, Go to Album.
    pub(crate) fn cover_menu(&mut self, ui: &mut Ui, path: &str, go_album: bool) {
        if ui.button(tr!("Cover…")).clicked() {
            self.open_cover_window(path);
            ui.close();
        }
        if ui.button(tr!("Change Cover…")).clicked() {
            self.change_cover(path);
            ui.close();
        }
        self.search_cover_menu(ui, path);
        if go_album {
            if let Some(key) = self.lib.album_of(path).map(|a| a.key.clone()) {
                ui.separator();
                if ui.button(tr!("Go to Album")).clicked() {
                    self.go(Page::Album(key));
                    ui.close();
                }
            }
        }
    }

    /// Change Cover… of a cover's own menu: the Cover window, and its file chooser at once.
    pub fn change_cover(&mut self, path: &str) {
        self.open_cover_window(path);
        if let Some(w) = self.cover_win.as_mut().filter(|w| w.path == path && !w.busy) {
            w.pick_now = true;
        }
    }
}

#[cfg(test)]
mod tests {
    //! The turning jacket and the cover search inside the whole player, headless, at
    //! 1024 x 768: the turn's geometry, the back inside the cover's square (mirrored and
    //! blurred, the album's songs, scrolling), the dots, a drag that follows the pointer
    //! and springs back or finishes, the keys, a click opening the album, the cover
    //! menus, and Search Cover Online → MusicBrainz… against a small MusicBrainz and
    //! Cover Art Archive on 127.0.0.1 (never the real ones).
    use super::*;
    use crate::player::Player;
    use realhires::settings::Settings;
    use realhires::{musiclib, playerstate};
    use std::io::{Read, Write};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const SCREEN: egui::Vec2 = vec2(1024.0, 768.0);

    fn ffmpeg(args: &[&str]) {
        let st = Command::new(realhires::probe::tool("ffmpeg")).args(["-v", "error", "-y"]).args(args).status().unwrap();
        assert!(st.success(), "{args:?}");
    }

    fn song(dir: &std::path::Path, album: &str, title: &str, artist: &str, n: u32) -> String {
        let d = dir.join(album);
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join(format!("{n:02} {title}.flac"));
        let src = format!("sine=frequency={}:sample_rate=8000:duration=6", 300 + 20 * n);
        let meta = [format!("title={title}"), format!("artist={artist}"), format!("album={album}"), format!("track={n}")];
        let mut args: Vec<&str> = vec!["-f", "lavfi", "-i", &src];
        for m in &meta {
            args.extend(["-metadata", m.as_str()]);
        }
        let out = f.to_string_lossy().into_owned();
        args.push(&out);
        ffmpeg(&args);
        out
    }

    fn frame(ctx: &egui::Context, p: &mut Player, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), SCREEN)), events, ..Default::default() };
        ctx.run_ui(input, |ui| p.ui(ui)).drop_without_applying_deltas();
    }

    fn run_until(ctx: &egui::Context, p: &mut Player, what: &str, done: impl Fn(&Player) -> bool) {
        let t0 = Instant::now();
        while !done(p) {
            assert!(t0.elapsed() < Duration::from_secs(90), "timed out waiting for {what}");
            frame(ctx, p, vec![]);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn button(at: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() }
    }

    /// Move there, press, let go (each in its own frame), and move away.
    fn click(ctx: &egui::Context, p: &mut Player, at: Pos2) {
        frame(ctx, p, vec![egui::Event::PointerMoved(at)]);
        frame(ctx, p, vec![button(at, egui::PointerButton::Primary, true)]);
        frame(ctx, p, vec![button(at, egui::PointerButton::Primary, false)]);
        frame(ctx, p, vec![egui::Event::PointerMoved(pos2(5.0, 763.0))]);
    }

    fn click_stay(ctx: &egui::Context, p: &mut Player, at: Pos2, which: egui::PointerButton) {
        frame(ctx, p, vec![egui::Event::PointerMoved(at)]);
        frame(ctx, p, vec![button(at, which, true)]);
        frame(ctx, p, vec![button(at, which, false)]);
    }

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
    }

    /// Every text the last of a few frames drew, where, and in what colour.
    fn texts(ctx: &egui::Context, p: &mut Player) -> Vec<(String, Rect, Color32, egui::FontId)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, Rect, Color32, egui::FontId)>) {
            match shape {
                egui::Shape::Text(t) => {
                    let s = t.galley.job.sections.first();
                    let color = if t.override_text_color.is_some() { t.override_text_color.unwrap() } else { s.map(|s| s.format.color).unwrap_or(t.fallback_color) };
                    let color = if color == Color32::PLACEHOLDER { t.fallback_color } else { color };
                    out.push((t.galley.text().to_string(), t.visual_bounding_rect(), color, s.map(|s| s.format.font_id.clone()).unwrap_or_default()));
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        for _ in 0..2 {
            frame(ctx, p, vec![]);
        }
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), SCREEN)), ..Default::default() };
        let output = ctx.run_ui(input, |ui| p.ui(ui));
        let mut out = vec![];
        output.shapes.iter().for_each(|s| walk(&s.shape, &mut out));
        output.drop_without_applying_deltas();
        out
    }

    fn names(t: &[(String, Rect, Color32, egui::FontId)]) -> Vec<String> {
        t.iter().map(|x| x.0.clone()).collect()
    }

    fn new_texts(before: &[String], after: &[String]) -> Vec<String> {
        let mut rest: Vec<&String> = before.iter().collect();
        let mut out = vec![];
        for t in after {
            if t.starts_with("Analysing moods") || t.starts_with("ムードを解析中") {
                continue;
            }
            match rest.iter().position(|b| *b == t) {
                Some(i) => {
                    rest.remove(i);
                }
                None => out.push(t.clone()),
            }
        }
        out
    }

    fn at(t: &[(String, Rect, Color32, egui::FontId)], text: &str) -> Pos2 {
        t.iter().rev().find(|x| x.0 == text).unwrap_or_else(|| panic!("no {text:?} in {:?}", names(t))).1.center()
    }

    #[test]
    fn a_turn_stays_inside_the_square_and_has_its_perspective() {
        let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(320.0, 320.0));
        let mut widths = vec![];
        for dir in [-1, 1] {
            for k in 0..=40 {
                let p = k as f64 / 40.0;
                let f = core::turn_frame(p, dir);
                let mesh = turn_mesh(rect, &f, None, Color32::from_rgb(200, 100, 50));
                let b = Rect::from_points(&mesh.vertices.iter().map(|v| v.pos).collect::<Vec<_>>());
                assert!(rect.expand(0.01).contains_rect(b), "{p} {dir}: {b:?} leaves {rect:?}");
                assert!((b.center().x - rect.center().x).abs() < 0.01, "it turns about its middle");
                assert_eq!(mesh.indices.len() % 3, 0);
                assert!(mesh.indices.iter().all(|i| (*i as usize) < mesh.vertices.len()));
                // the far edge is the shorter one
                let tall = |x: f32| {
                    let ys: Vec<f32> = mesh.vertices.iter().filter(|v| (v.pos.x - x).abs() < b.width() * 0.02 + 0.3).map(|v| v.pos.y).collect();
                    ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min)
                };
                if f.width > 0.3 && (0.05..0.95).contains(&p) {
                    let (left, right) = (tall(b.left()), tall(b.right()));
                    assert_eq!(right < left, f.far_right, "{p} {dir}: {left} {right}");
                }
                // darkened while it is turned away, never lighter than the colour
                let c = mesh.vertices[0].color;
                assert!(c.r() <= 200 && (p == 0.0 || p == 1.0 || c.r() < 200), "{p}: {c:?}");
                if dir == 1 {
                    widths.push(b.width());
                }
            }
        }
        assert!(widths[0] > 319.0 && widths[40] > 319.0 && widths[20] < 4.0, "full, edge-on, full: {widths:?}");
        assert!(widths[..20].windows(2).all(|w| w[1] <= w[0]) && widths[20..].windows(2).all(|w| w[1] >= w[0]));
        // a textured face keeps the picture's own corners at the outline (not mirrored again)
        let mesh = turn_mesh(rect, &core::turn_frame(0.0, 1), Some(egui::TextureId::Managed(7)), Color32::WHITE);
        assert!(mesh.vertices.iter().all(|v| (0.0..=1.0).contains(&v.uv.x) && (0.0..=1.0).contains(&v.uv.y)));
        let left = mesh.vertices.iter().min_by(|a, b| a.pos.x.total_cmp(&b.pos.x)).unwrap();
        assert!(left.uv.x < 0.01, "{left:?}");
        assert_eq!(mesh.vertices[0].color, Color32::WHITE);
    }

    /// A MusicBrainz and a Cover Art Archive in one small server: the search answers with
    /// three releases, two of which have a front cover.
    struct Fake {
        url: String,
        seen: Arc<Mutex<Vec<(String, String)>>>,
        most: Arc<AtomicUsize>,
        full: Vec<u8>,
    }

    fn picture(size: usize) -> Vec<u8> {
        let mut img = realhires::imaging::Rgb::new(size, size, 30);
        for y in 0..size {
            for x in 0..size {
                img.set(x, y, if x < size / 2 { [200, 40, 40] } else { [40, 60, 220] });
            }
        }
        realhires::imaging::to_png(&img)
    }

    fn fake_services() -> Fake {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let most = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let (thumb, full) = (picture(250), picture(600));
        let (seen2, most2, full2) = (seen.clone(), most.clone(), full.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (seen, most, active, thumb, full) = (seen2.clone(), most2.clone(), active.clone(), thumb.clone(), full2.clone());
                std::thread::spawn(move || {
                    let mut buf = [0u8; 8192];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                    let agent = request.lines().find(|l| l.to_lowercase().starts_with("user-agent:")).map(|l| l[11..].trim().to_string()).unwrap_or_default();
                    seen.lock().unwrap().push((path.clone(), agent));
                    let covers = path.starts_with("/release/") || path.starts_with("/img/");
                    if covers {
                        most.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                    }
                    let ok = |ctype: &str, body: &[u8]| {
                        let mut out = format!("HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
                        out.extend_from_slice(body);
                        out
                    };
                    let parts: Vec<&str> = path.split('?').next().unwrap_or("").trim_matches('/').split('/').collect();
                    let answer = match parts.as_slice() {
                        ["ws", "2", "release"] => {
                            let rel = |id: &str, date: &str, country: &str, format: &str| {
                                format!(
                                    r#"{{"id":"{id}","title":"Night Drive","status":"Official","date":"{date}","country":"{country}","score":100,"track-count":14,"artist-credit":[{{"name":"The Example Band","joinphrase":"","artist":{{"id":"a1","name":"The Example Band"}}}}],"release-group":{{"id":"rg1","primary-type":"Album","secondary-types":[]}},"media":[{{"position":1,"format":"{format}","track-count":14}}]}}"#
                                )
                            };
                            let body = format!(
                                r#"{{"created":"2026-11-25T00:00:00Z","count":3,"offset":0,"releases":[{},{},{}]}}"#,
                                rel("rel-one", "2001-03-07", "JP", "CD"),
                                rel("rel-none", "2003", "", "Cassette"),
                                rel("rel-two", "2015-10-01", "XW", "CD")
                            );
                            ok("application/json", body.as_bytes())
                        }
                        ["release", id @ ("rel-one" | "rel-two"), kind @ ("front" | "front-250")] => {
                            format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: /img/{id}-{kind}.png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
                        }
                        ["img", name] => {
                            std::thread::sleep(Duration::from_millis(60));
                            ok("image/png", if name.ends_with("-front-250.png") { &thumb } else { &full })
                        }
                        _ => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                    };
                    let _ = stream.write_all(&answer);
                    let _ = stream.flush();
                    if covers {
                        active.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            }
        });
        Fake { url, seen, most, full }
    }

    #[test]
    fn the_jacket_turns_over_and_covers_come_from_musicbrainz() {
        let _env = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        realhires::i18n::set_thread_language("English");
        let tmp = std::env::temp_dir().join(format!("rhr-player-jacket-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let music = tmp.join("Music");
        let fake = fake_services();
        std::env::set_var("REALHIRES_RESULTS", tmp.join("results.sqlite"));
        std::env::set_var("REALHIRES_SETTINGS", tmp.join("settings.json"));
        std::env::set_var(musiclib::LIBRARY_ENV, tmp.join("library.sqlite"));
        std::env::set_var(playerstate::PLAYER_ENV, tmp.join("player.json"));
        std::env::set_var(crate::audio::BACKEND_ENV, "null");
        std::env::set_var(realhires::themes::THEMES_ENV, tmp.join("themes.json"));
        std::env::set_var(realhires::smarttabs::TABS_ENV, tmp.join("tabs"));
        std::env::set_var(realhires::covers::COVERS_ENV, tmp.join("covers"));
        std::env::set_var(realhires::coverart::SERVER_ENV, &fake.url);
        std::env::set_var(realhires::musicbrainz::INTERVAL_ENV, "0");
        std::env::remove_var(core::NO_ANIMATION_ENV); // the turn takes its time here
        let songs: Vec<String> = (1..=14).map(|n| song(&music, "Night Drive", &format!("Song Number {n}"), "The Example Band", n)).collect();
        let plain = song(&music, "Plain", "No Cover Here", "Someone", 1);
        // the cover: yellow on the left, dark blue on the right
        let cover_file = music.join("Night Drive").join("cover.png");
        ffmpeg(&["-f", "lavfi", "-i", "color=c=0x101060:s=300x300:d=1,drawbox=x=0:y=0:w=150:h=300:color=0xf0d020:t=fill", "-frames:v", "1", &cover_file.to_string_lossy()]);
        let mut cfg = Settings::default();
        cfg.library_folders = vec![music.to_string_lossy().into_owned()];
        cfg.musicbrainz_server = fake.url.clone();
        cfg.analyse_moods = false;
        realhires::settings::save(&cfg, None).unwrap();
        let ctx = egui::Context::default();
        let mut p = Player::new(&ctx, cfg, vec![]);
        run_until(&ctx, &mut p, "the library index", |p| p.lib.tracks.len() == 15);
        let album = p.lib.albums.iter().find(|a| a.title == "Night Drive").cloned().unwrap();
        p.play_list(songs.clone(), Some(&songs[1]), "Night Drive");
        run_until(&ctx, &mut p, "playback", |p| p.engine.position().is_some());
        if p.is_playing() {
            p.toggle_play();
        }
        p.go(Page::NowPlaying);
        run_until(&ctx, &mut p, "the cover and its back", |p| p.jacket.ready(&album.key) && p.np_hits.cover.is_some());
        frame(&ctx, &mut p, vec![]);
        let cover = p.np_hits.cover.unwrap();
        assert!(cover.width() >= 120.0 && (cover.width() - cover.height()).abs() < 0.5, "{cover:?}");
        assert!(cover.right() < 224.0 + (SCREEN.x - 224.0) / 2.0 + 1.0, "in the left half at 1024 x 768: {cover:?}");
        assert_eq!((p.jacket.shows, p.jacket.painted), ("cover", Some(cover)));
        assert!(p.np_hits.album_songs.is_empty() && p.np_hits.dots.len() == 2);

        // ---- a dot turns the jacket: frames of the turn, each inside the square, then the back
        let frames = p.jacket.frames;
        let dot = p.np_hits.dots[1].center();
        click(&ctx, &mut p, dot);
        assert!(p.np_songs, "the songs are chosen at once; the turn shows it");
        let (mut seen, mut narrow) = (vec![], f64::MAX);
        let t0 = Instant::now();
        while p.jacket.turning() {
            assert!(t0.elapsed() < Duration::from_secs(60), "the turn never ended");
            frame(&ctx, &mut p, vec![]);
            if p.jacket.shows == "turn" {
                let b = p.jacket.painted.expect("a frame of the turn");
                assert!(cover.expand(0.5).contains_rect(b), "{b:?} leaves the cover's square {cover:?}");
                let f = p.jacket.last_frame.unwrap();
                seen.push(f.other);
                narrow = narrow.min(f.width);
                assert!(p.np_hits.album_songs.is_empty(), "the faces are pictures only during the turn");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(p.jacket.frames > frames, "no frame of the turn was drawn");
        assert!(seen.windows(2).all(|w| w[0] <= w[1]), "first the cover, then its back: {seen:?}");
        if seen.len() >= 6 {
            // (a machine that drew this many frames saw the jacket get narrow on the way)
            assert!(narrow < 0.9 && seen.contains(&true) && seen.contains(&false), "{narrow} {seen:?}");
        }
        frame(&ctx, &mut p, vec![]);
        assert_eq!((p.jacket.shows, p.jacket.painted), ("songs", Some(cover)));

        // ---- the back: inside the square, white on the blurred cover, the song playing bold
        let rows = p.np_hits.album_songs.clone();
        let layout = core::back_layout(cover.width().round() as i64);
        assert!(layout.rows < 14, "this album has more songs than fit ({} rows)", layout.rows);
        assert!(!rows.is_empty() && rows.len() as i64 <= layout.rows + 1, "{} rows", rows.len());
        assert!(rows.iter().all(|r| cover.contains_rect(r.0)), "a row leaves the square: {rows:?} in {cover:?}");
        assert_eq!(rows.iter().filter(|r| r.2).map(|r| r.1.clone()).collect::<Vec<_>>(), vec![songs[1].clone()]);
        let t = texts(&ctx, &mut p);
        let inside: Vec<_> = t.iter().filter(|x| cover.intersects(x.1)).cloned().collect();
        assert!(inside.iter().all(|x| cover.expand(1.0).contains_rect(x.1)), "a text leaves the square");
        let named = |s: &str| inside.iter().find(|x| x.0 == s).cloned().unwrap_or_else(|| panic!("{s:?} is not on the back: {:?}", names(&inside)));
        assert_eq!(named("Night Drive").2, Color32::WHITE);
        assert_eq!(named("Night Drive").3.family, look::title(13.0).family, "the album's title in bold");
        let artist = named("The Example Band");
        assert!(artist.2.a() < 255 && artist.2.a() > 120 && artist.1.top() > named("Night Drive").1.top(), "{artist:?}");
        assert_eq!(named("Song Number 2").3.family, look::title(13.0).family, "the song playing in bold");
        assert_eq!(named("Song Number 1").3.family, look::text(13.0).family);
        assert_eq!(named("Song Number 1").2, Color32::WHITE);
        assert!(inside.iter().any(|x| x.0 == "0:06" && x.2.a() < 255), "the times, in the secondary white");
        let bar = p.jacket.bar.expect("more songs than fit: a thin bar");
        assert!(cover.contains_rect(bar) && bar.width() <= 3.5 && bar.right() > cover.right() - 8.0, "{bar:?}");
        // its picture: the cover mirrored (yellow was on the left, so the back is light on the right)
        let back = super::make_back(&songs).expect("the back's picture");
        let px = |x: usize| back.pixels[back.size[0] * (back.size[1] / 2) + x];
        let (l, r) = (px(back.size[0] / 8), px(back.size[0] * 7 / 8));
        assert!(r.r() as i32 > l.r() as i32 + 40, "mirrored: {l:?} {r:?}");
        assert!(r.r() < 140, "under the scrim: {r:?}");
        let mid = px(back.size[0] / 2);
        assert!(mid.r() > l.r() + 15 && mid.r() + 15 < r.r(), "blurred across the middle: {l:?} {mid:?} {r:?}");

        // ---- the wheel scrolls the songs inside the square
        let over = pos2(cover.center().x, cover.top() + layout.list_top as f32 + layout.row_h as f32 * 1.5);
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(over)]);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -200.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Default::default(),
        };
        frame(&ctx, &mut p, vec![wheel]);
        run_until(&ctx, &mut p, "the songs to scroll", |p| p.jacket.scroll.0 > 100.0);
        for _ in 0..40 {
            frame(&ctx, &mut p, vec![]); // (egui keeps scrolling for a few frames)
            std::thread::sleep(Duration::from_millis(5));
        }
        let rows = p.np_hits.album_songs.clone();
        assert!(rows.iter().all(|r| cover.contains_rect(r.0)) && rows.iter().all(|r| r.1 != songs[0]), "{rows:?}");
        assert!(p.jacket.bar.unwrap().top() > bar.top() + 1.0 && cover.contains_rect(p.jacket.bar.unwrap()));
        assert!(p.np_songs && p.page == Page::NowPlaying);
        // a double-click plays that song with the album as the list
        let row = rows.iter().find(|r| r.0.height() > layout.row_h as f32 - 1.0 && !r.2).cloned().expect("a whole row");
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(row.0.center())]);
        for pressed in [true, false, true, false] {
            frame(&ctx, &mut p, vec![button(row.0.center(), egui::PointerButton::Primary, pressed)]);
        }
        run_until(&ctx, &mut p, "the double-clicked song", |p| p.now.as_ref().is_some_and(|n| n.path == row.1));
        assert_eq!(p.queue_name, "Night Drive");
        if p.is_playing() {
            p.toggle_play();
        }
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(pos2(5.0, 763.0))]);
        for _ in 0..30 {
            frame(&ctx, &mut p, vec![]);
        }

        // ---- a drag turns the jacket with the pointer: let go early and it springs back
        let c = cover.center();
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(c)]);
        frame(&ctx, &mut p, vec![button(c, egui::PointerButton::Primary, true)]);
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(c + vec2(30.0, 2.0))]);
        frame(&ctx, &mut p, vec![]);
        assert_eq!(p.jacket.shows, "turn");
        let got = p.jacket.progress().expect("the turn follows the pointer");
        assert!((got - 30.0 / cover.width() as f64).abs() < 0.01, "{got}");
        assert!(!p.jacket.last_frame.unwrap().far_right || p.jacket.last_frame.unwrap().far < 1.0);
        assert!(p.np_songs, "30 px is not a turn yet");
        frame(&ctx, &mut p, vec![button(c + vec2(30.0, 2.0), egui::PointerButton::Primary, false)]);
        run_until(&ctx, &mut p, "the jacket to spring back", |p| !p.jacket.turning());
        frame(&ctx, &mut p, vec![]);
        assert!(p.np_songs && p.jacket.shows == "songs", "let go at 30 px: it springs back");
        // half the cover's width is a quarter turn (edge-on); past it the other face shows
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(c)]);
        frame(&ctx, &mut p, vec![button(c, egui::PointerButton::Primary, true)]);
        let far = cover.width() * 0.7;
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(c + vec2(far * 0.5, 0.0))]);
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(c + vec2(far, 3.0))]);
        frame(&ctx, &mut p, vec![]);
        let f = p.jacket.last_frame.unwrap();
        assert!((p.jacket.progress().unwrap() - 0.7).abs() < 0.01 && f.other, "{f:?}");
        assert!(cover.expand(0.5).contains_rect(p.jacket.painted.unwrap()));
        assert!(p.np_songs, "not before the pointer lets go");
        frame(&ctx, &mut p, vec![button(c + vec2(far, 3.0), egui::PointerButton::Primary, false)]);
        assert!(!p.np_songs, "let go past 60 px: the jacket turns over");
        run_until(&ctx, &mut p, "the turn to finish", |p| !p.jacket.turning());
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(pos2(5.0, 763.0))]);
        frame(&ctx, &mut p, vec![]);
        assert_eq!((p.jacket.shows, p.page.clone()), ("cover", Page::NowPlaying), "a drag is not a click");

        // ---- the keys turn it while the pointer is over it, and only then
        frame(&ctx, &mut p, vec![key(egui::Key::ArrowRight)]);
        assert!(!p.np_songs, "the pointer is elsewhere");
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(c)]);
        frame(&ctx, &mut p, vec![key(egui::Key::ArrowRight)]);
        assert!(p.np_songs);
        run_until(&ctx, &mut p, "the turn", |p| !p.jacket.turning());
        frame(&ctx, &mut p, vec![key(egui::Key::ArrowLeft)]);
        assert!(!p.np_songs);
        // turned again in the middle of the turn: it goes back from where it is
        frame(&ctx, &mut p, vec![key(egui::Key::ArrowLeft)]);
        assert!(p.np_songs && p.jacket.turning());
        run_until(&ctx, &mut p, "the turn", |p| !p.jacket.turning());
        frame(&ctx, &mut p, vec![key(egui::Key::ArrowRight)]);
        run_until(&ctx, &mut p, "the turn", |p| !p.jacket.turning());
        assert!(!p.np_songs);
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(pos2(5.0, 763.0))]);
        for _ in 0..30 {
            frame(&ctx, &mut p, vec![]);
        }

        // ---- the cover's menu, and a click that opens the album
        let before = names(&texts(&ctx, &mut p));
        click_stay(&ctx, &mut p, c, egui::PointerButton::Secondary);
        let after = texts(&ctx, &mut p);
        assert_eq!(new_texts(&before, &names(&after)), ["Cover…", "Change Cover…", "Search Cover Online", "Go to Album"]);
        click_stay(&ctx, &mut p, at(&after, "Go to Album"), egui::PointerButton::Primary);
        frame(&ctx, &mut p, vec![]);
        assert_eq!(p.page, Page::Album(album.key.clone()));
        p.go(Page::NowPlaying);
        for _ in 0..30 {
            frame(&ctx, &mut p, vec![]);
        }
        click(&ctx, &mut p, c);
        assert_eq!(p.page, Page::Album(album.key.clone()), "a click on the cover opens the album");
        assert!(p.cover_win.is_none());
        // the album page's cover: the same menu without Go to Album, and a click opens the Cover window
        for _ in 0..30 {
            frame(&ctx, &mut p, vec![]);
        }
        let page_cover = p.album_cover_rect.expect("the album page's cover").center();
        click(&ctx, &mut p, page_cover);
        assert!(p.cover_win.as_ref().is_some_and(|w| w.target_paths().len() == 14), "a click opens the Cover window");
        assert_eq!(p.page, Page::Album(album.key.clone()));
        p.cover_win = None;
        for _ in 0..30 {
            frame(&ctx, &mut p, vec![]);
        }
        let before = names(&texts(&ctx, &mut p));
        click_stay(&ctx, &mut p, page_cover, egui::PointerButton::Secondary);
        let after = texts(&ctx, &mut p);
        assert_eq!(new_texts(&before, &names(&after)), ["Cover…", "Change Cover…", "Search Cover Online"]);
        click_stay(&ctx, &mut p, at(&after, "Cover…"), egui::PointerButton::Primary);
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(pos2(5.0, 763.0))]);
        assert!(p.cover_win.is_some(), "Cover… opens the Cover window");

        // ---- Search Cover Online → MusicBrainz…: the releases' covers from the archive
        run_until(&ctx, &mut p, "the cover read", |p| p.cover_win.as_ref().is_some_and(|w| w.idle()));
        let shown = texts(&ctx, &mut p);
        click_stay(&ctx, &mut p, at(&shown, "Search Cover Online"), egui::PointerButton::Primary);
        let menu = texts(&ctx, &mut p);
        assert_eq!(new_texts(&names(&shown), &names(&menu)), ["MusicBrainz…", "Google Images"]);
        click_stay(&ctx, &mut p, at(&menu, "MusicBrainz…"), egui::PointerButton::Primary);
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(pos2(5.0, 763.0))]);
        let search = |p: &Player| p.cover_win.as_ref().and_then(|w| w.search.as_ref()).map(|s| (s.done, s.tiles.len(), s.releases.len(), s.status_text()));
        assert!(search(&p).is_some(), "MusicBrainz… opens the cover search");
        {
            let s = p.cover_win.as_ref().unwrap().search.as_ref().unwrap();
            assert_eq!((s.album.as_str(), s.artist.as_str()), ("Night Drive", "The Example Band"));
        }
        run_until(&ctx, &mut p, "the covers", |p| search(p).is_some_and(|s| s.0));
        assert_eq!(search(&p).unwrap(), (true, 2, 3, "2 covers found. Click one to see it large.".to_string()));
        let t = texts(&ctx, &mut p);
        let s = p.cover_win.as_ref().unwrap().search.as_ref().unwrap();
        let (win, tiles) = (s.rect.expect("the window"), s.tile_rects.clone());
        assert!(Rect::from_min_size(pos2(0.0, 0.0), SCREEN).contains_rect(win), "the window fits 1024 x 768: {win:?}");
        assert!(tiles.len() == 2 && tiles.iter().all(|r| win.contains_rect(*r)), "{tiles:?} in {win:?}");
        for want in ["Night Drive", "The Example Band", "2001 · JP · CD", "2015 · XW · CD", "Use this cover", "Back to the covers"] {
            let there = t.iter().any(|x| x.0 == want && win.contains_rect(x.1));
            assert_eq!(there, !matches!(want, "Use this cover" | "Back to the covers"), "{want}");
        }
        assert!(t.iter().any(|x| x.0.starts_with("Covers come from the Cover Art Archive")), "the credit");
        // a click shows it large with its size; Use this cover sets it as Change cover… does
        click(&ctx, &mut p, tiles[0].center());
        run_until(&ctx, &mut p, "the chosen cover", |p| p.cover_win.as_ref().and_then(|w| w.search.as_ref()).is_some_and(|s| s.picture.is_some()));
        let t = texts(&ctx, &mut p);
        assert!(t.iter().any(|x| x.0.starts_with("600 × 600 pixels · PNG")), "{:?}", names(&t));
        assert!(t.iter().any(|x| x.0 == "Night Drive — The Example Band  ·  2001 · JP · CD"));
        assert!(t.iter().any(|x| x.0 == "All 14 songs of this album") && t.iter().any(|x| x.0 == "This song only"));
        let mtimes: Vec<i64> = songs.iter().map(|s| p.lib.track(s).map(|t| t.mtime_ns).unwrap_or(0)).collect();
        click(&ctx, &mut p, at(&t, "Use this cover"));
        assert!(p.cover_win.as_ref().is_some_and(|w| w.search.is_none() && (w.busy || !w.summary.is_empty())), "the Cover window sets it");
        run_until(&ctx, &mut p, "the covers to be changed", |p| p.cover_win.as_ref().is_some_and(|w| w.idle() && !w.summary.is_empty()));
        run_until(&ctx, &mut p, "the library reading the changed songs", |p| {
            songs.iter().zip(&mtimes).all(|(s, m)| p.lib.track(s).is_some_and(|t| t.mtime_ns != *m))
        });
        assert_eq!(p.cover_win.as_ref().unwrap().summary, "Changed the cover of 14 songs.");
        assert_eq!(covers::cover_source(&[songs[3].clone()]).map(|s| (s.kind, s.data)), Some(("embedded", fake.full.clone())));
        // what was asked: the album's artist and title of MusicBrainz, the releases' ids of
        // the archive, each once, one download at a time, by name, and never a path
        let seen = fake.seen.lock().unwrap().clone();
        let paths: Vec<&str> = seen.iter().map(|s| s.0.as_str()).collect();
        assert_eq!(paths.len(), 8, "{paths:?}");
        assert!(paths[0].starts_with("/ws/2/release?query=release%3A%22Night+Drive%22+AND+artist%3A%22The+Example+Band%22&limit=12"), "{}", paths[0]);
        assert_eq!(
            paths[1..],
            ["/release/rel-one/front-250", "/img/rel-one-front-250.png", "/release/rel-none/front-250", "/release/rel-two/front-250", "/img/rel-two-front-250.png", "/release/rel-one/front", "/img/rel-one-front.png"]
        );
        assert!(seen.iter().all(|s| s.1 == realhires::musicbrainz::user_agent()), "{seen:?}");
        assert!(!seen.iter().any(|s| s.0.contains("Music") || s.0.contains("flac") || s.0.contains("Song")), "a path or a song was sent");
        assert_eq!(fake.most.load(Ordering::SeqCst), 1, "one download at a time");
        assert!(crate::web::fake::opened().is_empty(), "no browser");

        // ---- in Japanese; a song without a cover turns too (a plain sleeve)
        p.cover_win = None;
        realhires::i18n::set_thread_language("日本語");
        p.play_list(vec![plain.clone()], Some(&plain), "Plain");
        run_until(&ctx, &mut p, "playback", |p| p.now.as_ref().is_some_and(|n| n.path == plain) && p.engine.position().is_some());
        if p.is_playing() {
            p.toggle_play();
        }
        p.go(Page::NowPlaying);
        for _ in 0..30 {
            frame(&ctx, &mut p, vec![]);
        }
        let cover = p.np_hits.cover.unwrap();
        let before = names(&texts(&ctx, &mut p));
        click_stay(&ctx, &mut p, cover.center(), egui::PointerButton::Secondary);
        let after = texts(&ctx, &mut p);
        assert_eq!(new_texts(&before, &names(&after)), ["アートワーク…", "アートワークを変更…", "オンラインでアートワークを検索", "アルバムへ移動"]);
        click_stay(&ctx, &mut p, at(&after, "アルバムへ移動"), egui::PointerButton::Primary);
        frame(&ctx, &mut p, vec![egui::Event::PointerMoved(pos2(5.0, 763.0))]);
        assert!(matches!(p.page, Page::Album(_)));
        p.go(Page::NowPlaying);
        for _ in 0..30 {
            frame(&ctx, &mut p, vec![]);
        }
        p.set_np_side(true);
        let t0 = Instant::now();
        while p.jacket.turning() {
            assert!(t0.elapsed() < Duration::from_secs(60));
            frame(&ctx, &mut p, vec![]);
            assert!(p.jacket.painted.is_none_or(|b| cover.expand(0.5).contains_rect(b)));
            std::thread::sleep(Duration::from_millis(5));
        }
        frame(&ctx, &mut p, vec![]);
        assert_eq!(p.jacket.shows, "songs");
        assert_eq!(p.np_hits.album_songs.iter().map(|r| (r.1.clone(), r.2)).collect::<Vec<_>>(), vec![(plain.clone(), true)]);
        assert!(p.jacket.bar.is_none(), "one song: nothing to scroll");
        realhires::i18n::set_thread_language("English");
        std::env::remove_var(realhires::coverart::SERVER_ENV);
        std::env::remove_var(realhires::musicbrainz::INTERVAL_ENV);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
