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
