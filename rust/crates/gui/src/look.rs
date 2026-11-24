//! Fonts and colours of the player (2.0, `docs/player-spec.md`): SF Pro from
//! the system on macOS (bold for headings, "SF Pro Display Bold"), the bundled
//! Inter (SIL Open Font License) on Windows and Linux, or the family chosen in
//! Settings → Appearance; light and dark palettes close to Apple Music's.

use std::sync::{Arc, OnceLock};

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle};

/// The variable Inter, with its licence in `realhires/assets/fonts/`.
const INTER: &[u8] = include_bytes!("../../../../realhires/assets/fonts/InterVariable.ttf");
const SF_PRO: &str = "/System/Library/Fonts/SFNS.ttf";

/// The family name of the bold heading font.
pub const BOLD: &str = "rhr-bold";

pub fn bold() -> FontFamily {
    FontFamily::Name(BOLD.into())
}

/// Heading sizes.
pub fn title(size: f32) -> FontId {
    FontId::new(size, bold())
}

pub fn text(size: f32) -> FontId {
    FontId::proportional(size)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub text: Color32,
    pub secondary: Color32,
    pub separator: Color32,
    pub accent: Color32,
    pub lcd: Color32,
    pub hover: Color32,
    pub dark: bool,
}

fn hex(c: u32) -> Color32 {
    crate::style::hex(c)
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: hex(0x1e1e1e),
            sidebar: hex(0x262628),
            text: hex(0xf5f5f7),
            secondary: hex(0xa1a1a6),
            separator: hex(0x3a3a3c),
            accent: hex(0xfa2d48),
            lcd: hex(0x2c2c2e),
            hover: hex(0x333336),
            dark,
        }
    } else {
        Palette {
            bg: hex(0xffffff),
            sidebar: hex(0xf5f5f7),
            text: hex(0x1d1d1f),
            secondary: hex(0x6e6e73),
            separator: hex(0xd2d2d7),
            accent: hex(0xfa2d48),
            lcd: hex(0xf2f2f4),
            hover: hex(0xe8e8ed),
            dark,
        }
    }
}

pub fn current(ctx: &egui::Context) -> Palette {
    palette(ctx.global_style().visuals.dark_mode)
}

/// The installed font families (for the menu in Settings), read once.
pub fn installed_families() -> Vec<String> {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    FAMILIES
        .get_or_init(|| {
            let mut c = fontique::Collection::new(fontique::CollectionOptions { shared: false, system_fonts: true });
            let mut v: Vec<String> =
                c.family_names().filter(|n| !n.starts_with('.')).map(str::to_string).collect();
            v.sort_by_key(|n| n.to_lowercase());
            v.dedup();
            v
        })
        .clone()
}

fn tweak(weight: f32, opsz: Option<f32>) -> egui::FontTweak {
    let mut coords = egui::epaint::text::VariationCoords::new([(b"wght", weight)]);
    if let Some(o) = opsz {
        coords.push(b"opsz", o);
    }
    egui::FontTweak { coords, ..Default::default() }
}

/// (regular, bold) font data for `family` ("" = SF Pro on macOS, Inter elsewhere).
fn font_pair(family: &str) -> (FontData, FontData, String) {
    if !family.is_empty() {
        let mut c = fontique::Collection::new(fontique::CollectionOptions { shared: false, system_fonts: true });
        if let Some(fam) = c.family_by_name(family) {
            let pick = |w: fontique::FontWeight| {
                fam.match_font(fontique::FontWidth::NORMAL, fontique::FontStyle::Normal, w, false).and_then(|f| {
                    let blob = f.load(None)?;
                    let data = FontData::from_blob(Arc::new(blob.data().to_vec()), f.index());
                    let variable = f.has_weight_axis();
                    Some(if variable { data.tweak(tweak(w.value(), None)) } else { data })
                })
            };
            if let (Some(r), Some(b)) = (pick(fontique::FontWeight::NORMAL), pick(fontique::FontWeight::BOLD)) {
                return (r, b, family.to_string());
            }
        }
    }
    if cfg!(target_os = "macos") {
        if let Ok(bytes) = std::fs::read(SF_PRO) {
            let bytes = Arc::new(bytes);
            let r = FontData::from_blob(bytes.clone(), 0).tweak(tweak(400.0, Some(17.0)));
            let b = FontData::from_blob(bytes, 0).tweak(tweak(700.0, Some(28.0)));
            return (r, b, "SF Pro".into());
        }
    }
    let r = FontData::from_static(INTER).tweak(tweak(400.0, Some(14.0)));
    let b = FontData::from_static(INTER).tweak(tweak(700.0, Some(32.0)));
    (r, b, "Inter".into())
}

/// Install the fonts and text sizes; returns the family actually used.
pub fn install_fonts(ctx: &egui::Context, family: &str) -> String {
    let (regular, bold_data, used) = font_pair(family);
    let mut defs = FontDefinitions::default();
    defs.font_data.insert("rhr-regular".into(), Arc::new(regular));
    defs.font_data.insert(BOLD.into(), Arc::new(bold_data));
    let fallbacks = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    defs.families.entry(FontFamily::Proportional).or_default().insert(0, "rhr-regular".into());
    let mut heading = vec![BOLD.to_string()];
    heading.extend(fallbacks);
    defs.families.insert(bold(), heading);
    ctx.set_fonts(defs);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, FontId::new(16.0, bold())),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
    });
    used
}

/// Light and dark visuals with the palette's colours, and the theme setting.
pub fn install_theme(ctx: &egui::Context, theme: &str) {
    for dark in [false, true] {
        let p = palette(dark);
        let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = p.bg;
        v.window_fill = p.bg;
        v.extreme_bg_color = if dark { hex(0x161618) } else { hex(0xffffff) };
        v.faint_bg_color = if dark { hex(0x232325) } else { hex(0xf7f7f9) };
        v.override_text_color = None;
        v.hyperlink_color = p.accent;
        v.selection.bg_fill = p.accent.gamma_multiply(if dark { 0.55 } else { 0.85 });
        v.selection.stroke.color = Color32::WHITE;
        v.window_corner_radius = egui::CornerRadius::same(12);
        v.menu_corner_radius = egui::CornerRadius::same(8);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = egui::CornerRadius::same(6);
        }
        v.widgets.noninteractive.bg_stroke.color = p.separator;
        v.widgets.inactive.weak_bg_fill = if dark { hex(0x3a3a3c) } else { hex(0xececf0) };
        v.widgets.inactive.bg_fill = v.widgets.inactive.weak_bg_fill;
        v.widgets.hovered.weak_bg_fill = p.hover;
        v.widgets.hovered.bg_fill = p.hover;
        v.slider_trailing_fill = true;
        ctx.set_visuals_of(if dark { egui::Theme::Dark } else { egui::Theme::Light }, v);
    }
    ctx.set_theme(match theme {
        "Light" => egui::ThemePreference::Light,
        "Dark" => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    });
}
