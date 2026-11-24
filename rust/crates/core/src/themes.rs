//! Colour themes of the player (2.0), the counterpart of
//! `realhires/themes.py`: the same built-in themes and the same `themes.json`
//! with the user's own, so both versions share them.

use std::path::{Path, PathBuf};

use crate::json::{self, Obj as ObjectBuilder, Value};
use crate::pyunicode::casefold;

pub const THEMES_ENV: &str = "REALHIRES_THEMES";
pub const FILE_NAME: &str = "themes.json";
pub const THEME_FILE_FORMAT: &str = "real-hi-res-theme";
pub const SYSTEM: &str = "Match the system";
pub const MAX_NAME: usize = 80;

pub const COLOR_KEYS: [&str; 8] = ["background", "sidebar", "text", "secondary", "separator", "accent", "panel", "hover"];

pub fn color_label(key: &str) -> &'static str {
    match key {
        "background" => "Background",
        "sidebar" => "Sidebar",
        "text" => "Text",
        "secondary" => "Secondary text",
        "separator" => "Lines",
        "accent" => "Accent",
        "panel" => "Playback panel",
        _ => "Highlight",
    }
}

/// (name, dark, colours in COLOR_KEYS order)
pub const BUILTIN: [(&str, bool, [&str; 8]); 8] = [
    ("Light", false, ["#ffffff", "#f5f5f7", "#1d1d1f", "#6e6e73", "#d2d2d7", "#fa2d48", "#f2f2f4", "#e8e8ed"]),
    ("Dark", true, ["#1e1e1e", "#262628", "#f5f5f7", "#a1a1a6", "#3a3a3c", "#fa2d48", "#2c2c2e", "#333336"]),
    ("Midnight", true, ["#0f1221", "#161a2e", "#e8eaf6", "#9aa0c3", "#2a2f4a", "#7c8cff", "#1c2138", "#23294a"]),
    ("Graphite", true, ["#1b1b1d", "#232326", "#ececec", "#9a9a9f", "#38383c", "#ff9f0a", "#2a2a2d", "#313135"]),
    ("Forest", true, ["#121a15", "#18231c", "#e6efe8", "#9db3a3", "#2b3a30", "#4cc38a", "#1d2a22", "#24342a"]),
    ("Ocean", false, ["#f4f8fb", "#e8f0f6", "#102a43", "#52677c", "#c8d6e2", "#0a84ff", "#e9f1f7", "#dbe7f1"]),
    ("Sepia", false, ["#f6efe2", "#ece2cf", "#3b2f22", "#7a6a55", "#d9cbb1", "#c0673a", "#efe5d2", "#e3d6bd"]),
    ("Rose", false, ["#fff7f9", "#fbeaf0", "#3a1d2a", "#8a6574", "#efd3de", "#e0457b", "#f9e6ee", "#f3dbe5"]),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    /// "#rrggbb" in COLOR_KEYS order
    pub colors: [String; 8],
}

impl Theme {
    pub fn color(&self, key: &str) -> &str {
        COLOR_KEYS.iter().position(|k| *k == key).map(|i| self.colors[i].as_str()).unwrap_or("#000000")
    }

    pub fn to_json(&self) -> Value {
        let mut colors = ObjectBuilder::new();
        for (k, v) in COLOR_KEYS.iter().zip(self.colors.iter()) {
            colors = colors.put(k, v.as_str());
        }
        ObjectBuilder::new().put("name", self.name.as_str()).put("dark", self.dark).put("colors", colors.build()).build()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeError(pub String);

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub fn builtin(name: &str) -> Option<Theme> {
    BUILTIN.iter().find(|(n, _, _)| *n == name).map(|(n, dark, c)| Theme {
        name: n.to_string(),
        dark: *dark,
        colors: c.map(str::to_string),
    })
}

pub fn default_path() -> PathBuf {
    std::env::var_os(THEMES_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::store::data_dir().join(FILE_NAME))
}

pub fn clean_name(name: &str) -> String {
    crate::playerstate::clean_name(name)
}

fn is_hex(v: &str) -> bool {
    v.len() == 7 && v.starts_with('#') && v[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A valid theme from stored data (Python `normalize`).
pub fn normalize(v: &Value) -> Option<Theme> {
    if !matches!(v, Value::Object(_)) {
        return None;
    }
    let name = clean_name(v.get("name").and_then(Value::as_str).unwrap_or(""));
    if name.is_empty() {
        return None;
    }
    let dark = v.get("dark").is_some_and(Value::truthy);
    let base = builtin(if dark { "Dark" } else { "Light" }).unwrap();
    let given = v.get("colors").filter(|c| matches!(c, Value::Object(_)));
    let mut colors = base.colors.clone();
    for (i, k) in COLOR_KEYS.iter().enumerate() {
        if let Some(s) = given.and_then(|g| g.get(k)).and_then(Value::as_str) {
            let s = crate::pyunicode::strip(s).to_lowercase();
            if is_hex(&s) {
                colors[i] = s;
            }
        }
    }
    Some(Theme { name, dark, colors })
}

pub fn load_custom(path: Option<&Path>) -> Vec<Theme> {
    let p = path.map(Path::to_path_buf).unwrap_or_else(default_path);
    let Some(data) = std::fs::read_to_string(&p).ok().and_then(|t| json::parse(&t).ok()) else { return vec![] };
    let mut seen: Vec<String> = BUILTIN.iter().map(|(n, _, _)| casefold(n)).collect();
    seen.push(casefold(SYSTEM));
    let mut out = vec![];
    if let Some(Value::Array(list)) = data.get("themes") {
        for t in list {
            if let Some(t) = normalize(t) {
                if !seen.contains(&casefold(&t.name)) {
                    seen.push(casefold(&t.name));
                    out.push(t);
                }
            }
        }
    }
    out
}

pub fn save_custom(themes: &[Theme], path: Option<&Path>) -> std::io::Result<PathBuf> {
    let p = path.map(Path::to_path_buf).unwrap_or_else(default_path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let data = ObjectBuilder::new()
        .put("_version", 1i64)
        .put("themes", Value::Array(themes.iter().map(Theme::to_json).collect()))
        .build();
    let style = json::Style { indent: Some(2), compact: false, ensure_ascii: false, sort_keys: true };
    let mut tmp = p.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, json::dumps(&data, style) + "\n")?;
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

/// Every theme name for the menu: Match the system, the built-in, the user's.
pub fn names(custom: &[Theme]) -> Vec<String> {
    let mut v = vec![SYSTEM.to_string()];
    v.extend(BUILTIN.iter().map(|(n, _, _)| n.to_string()));
    v.extend(custom.iter().map(|t| t.name.clone()));
    v
}

/// The theme to draw with; unknown names follow the system.
pub fn resolve(name: &str, system_dark: bool, custom: &[Theme]) -> Theme {
    if let Some(t) = builtin(name) {
        return t;
    }
    if let Some(t) = custom.iter().find(|t| t.name == name) {
        return t.clone();
    }
    builtin(if system_dark { "Dark" } else { "Light" }).unwrap()
}

pub fn check_name(name: &str, custom: &[Theme], renaming: &str) -> Result<String, ThemeError> {
    let clean = clean_name(name);
    if clean.is_empty() {
        return Err(ThemeError("Give the theme a name.".into()));
    }
    let key = casefold(&clean);
    if key == casefold(SYSTEM) || BUILTIN.iter().any(|(n, _, _)| casefold(n) == key) {
        return Err(ThemeError(format!("“{clean}” is the name of a built-in theme; choose another one.")));
    }
    if custom.iter().any(|t| casefold(&t.name) == key && t.name != renaming) {
        return Err(ThemeError(format!("You already have a theme called “{clean}”.")));
    }
    Ok(clean)
}

/// "Midnight copy", "Midnight copy 2"...
pub fn copy_name(base: &str, custom: &[Theme]) -> String {
    let taken: Vec<String> = names(custom).iter().map(|n| casefold(n)).collect();
    let mut name = clean_name(&format!("{base} copy"));
    let mut n = 2;
    while taken.contains(&casefold(&name)) {
        name = clean_name(&format!("{base} copy {n}"));
        n += 1;
    }
    name
}

pub fn export_theme(t: &Theme) -> String {
    let mut colors = ObjectBuilder::new();
    for (k, v) in COLOR_KEYS.iter().zip(t.colors.iter()) {
        colors = colors.put(k, v.as_str());
    }
    let data = ObjectBuilder::new()
        .put("format", THEME_FILE_FORMAT)
        .put("version", 1i64)
        .put("name", t.name.as_str())
        .put("dark", t.dark)
        .put("colors", colors.build())
        .build();
    let style = json::Style { indent: Some(2), compact: false, ensure_ascii: false, sort_keys: true };
    json::dumps(&data, style) + "\n"
}

/// A new custom theme from an exported file (a taken name gets " 2", " 3"...).
pub fn import_theme(text: &str, custom: &[Theme]) -> Result<Theme, ThemeError> {
    let data = json::parse(text.trim_start_matches('\u{feff}')).ok();
    let mut t = data
        .as_ref()
        .filter(|d| matches!(d.get("colors"), Some(Value::Object(_))))
        .and_then(normalize)
        .ok_or_else(|| ThemeError("This file is not a theme of Universal Music Player.".into()))?;
    let taken: Vec<String> = names(custom).iter().map(|n| casefold(n)).collect();
    let base = t.name.clone();
    let mut n = 2;
    while taken.contains(&casefold(&t.name)) {
        t.name = clean_name(&format!("{base} {n}"));
        n += 1;
    }
    Ok(t)
}

pub const FOLDER_NAME: &str = "themes";
pub const MAX_FILE_BYTES: u64 = 1 << 20;

/// A theme from a file in the themes folder.
#[derive(Clone, Debug, PartialEq)]
pub struct FileTheme {
    pub theme: Theme,
    pub file: PathBuf,
}

/// The themes folder: "themes" next to themes.json (Python `folder`).
pub fn folder(path: Option<&Path>) -> PathBuf {
    let p = path.map(Path::to_path_buf).unwrap_or_else(default_path);
    let abs = PathBuf::from(crate::paths::abspath(&p.to_string_lossy()));
    abs.parent().map(Path::to_path_buf).unwrap_or_default().join(FOLDER_NAME)
}

/// The themes of the .json files in the themes folder, in file-name order; a
/// taken name gets " 2", " 3"...; files that are not themes are left out
/// (Python `load_folder`).
pub fn load_folder(custom: &[Theme], folder_path: Option<&Path>) -> Vec<FileTheme> {
    let d = folder_path.map(Path::to_path_buf).unwrap_or_else(|| folder(None));
    let Ok(entries) = std::fs::read_dir(&d) else { return vec![] };
    let mut files: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.') && n.len() >= 5 && n[n.len() - 5..].eq_ignore_ascii_case(".json"))
        .collect();
    files.sort();
    let mut known: Vec<Theme> = custom.to_vec();
    let mut out = vec![];
    for n in files {
        let p = d.join(&n);
        let Ok(meta) = std::fs::metadata(&p) else { continue };
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Some(text) = std::fs::read(&p).ok().and_then(|b| String::from_utf8(b).ok()) else { continue };
        let Ok(t) = import_theme(&text, &known) else { continue };
        known.push(t.clone());
        out.push(FileTheme { theme: t, file: p });
    }
    out
}
