//! What the player remembers (2.0), the counterpart of
//! `realhires/playerstate.py`: the same `player.json` next to the saved
//! results, so both versions share likes, volume, modes and the equalizer.

use std::path::{Path, PathBuf};

use crate::eq::EqState;
use crate::json::{self, Obj as ObjectBuilder, Value};
use crate::playqueue::{REPEAT_MODES, SHUFFLE_MODES};

pub const PLAYER_ENV: &str = "REALHIRES_PLAYER";
pub const FILE_NAME: &str = "player.json";
pub const STATE_VERSION: i64 = 1;

pub fn default_path() -> PathBuf {
    std::env::var_os(PLAYER_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::store::data_dir().join(FILE_NAME))
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerState {
    pub liked: Vec<String>,
    pub volume: f64,
    pub muted: bool,
    pub shuffle: String,
    pub repeat: String,
    pub eq: EqState,
    pub last_path: String,
    pub last_position: f64,
}

impl Default for PlayerState {
    fn default() -> Self {
        PlayerState {
            liked: vec![],
            volume: 0.8,
            muted: false,
            shuffle: "off".into(),
            repeat: "off".into(),
            eq: EqState::default(),
            last_path: String::new(),
            last_position: 0.0,
        }
    }
}

impl PlayerState {
    pub fn is_liked(&self, path: &str) -> bool {
        self.liked.iter().any(|p| p == path)
    }

    pub fn set_liked(&mut self, path: &str, liked: bool) {
        self.liked.retain(|p| p != path);
        if liked {
            self.liked.push(path.to_string());
        }
        self.liked.sort();
        self.liked.dedup();
    }

    pub fn to_json(&self) -> Value {
        let mut liked = self.liked.clone();
        liked.sort();
        liked.dedup();
        ObjectBuilder::new()
            .put("_version", STATE_VERSION)
            .put("liked", Value::Array(liked.into_iter().map(Value::Str).collect()))
            .put("volume", self.volume)
            .put("muted", self.muted)
            .put("shuffle", self.shuffle.as_str())
            .put("repeat", self.repeat.as_str())
            .put("eq", self.eq.to_json())
            .put(
                "last",
                ObjectBuilder::new().put("path", self.last_path.as_str()).put("position", self.last_position).build(),
            )
            .build()
    }

    pub fn from_json(data: &Value) -> PlayerState {
        let mut st = PlayerState::default();
        if !matches!(data, Value::Object(_)) {
            return st;
        }
        if let Some(Value::Array(liked)) = data.get("liked") {
            let mut v: Vec<String> =
                liked.iter().filter_map(Value::as_str).filter(|p| !p.is_empty()).map(str::to_string).collect();
            v.sort();
            v.dedup();
            st.liked = v;
        }
        if let Some(v) = data.get("volume").and_then(as_num) {
            if !v.is_nan() {
                st.volume = v.max(0.0).min(1.0);
            }
        }
        st.muted = data.get("muted").is_some_and(Value::truthy);
        st.shuffle =
            data.get("shuffle").and_then(Value::as_str).filter(|m| SHUFFLE_MODES.contains(m)).unwrap_or("off").into();
        // "stop after this song" is for the song playing now, never for the next start
        st.repeat = data
            .get("repeat")
            .and_then(Value::as_str)
            .filter(|m| REPEAT_MODES.contains(m) && *m != "stop")
            .unwrap_or("off")
            .into();
        st.eq = EqState::from_json(data.get("eq").unwrap_or(&Value::Null));
        if let Some(last @ Value::Object(_)) = data.get("last") {
            st.last_path = last.get("path").and_then(Value::as_str).unwrap_or("").to_string();
            st.last_position = last.get("position").and_then(as_num).filter(|v| !v.is_nan()).unwrap_or(0.0).max(0.0);
        }
        st
    }
}

fn as_num(v: &Value) -> Option<f64> {
    match v {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Str(s) => crate::pyunicode::strip(s).parse().ok(),
        _ => None,
    }
}

pub fn load(path: Option<&Path>) -> PlayerState {
    let p = path.map(Path::to_path_buf).unwrap_or_else(default_path);
    match std::fs::read_to_string(&p).ok().and_then(|t| json::parse(&t).ok()) {
        Some(v) => PlayerState::from_json(&v),
        None => PlayerState::default(),
    }
}

pub fn save(state: &PlayerState, path: Option<&Path>) -> std::io::Result<PathBuf> {
    let p = path.map(Path::to_path_buf).unwrap_or_else(default_path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = p.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let style = json::Style { indent: Some(2), compact: false, ensure_ascii: false, sort_keys: true };
    std::fs::write(&tmp, json::dumps(&state.to_json(), style) + "\n")?;
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("rhr-player-{}", std::process::id()));
        let path = dir.join("player.json");
        let mut st = load(Some(&path));
        assert_eq!(st, PlayerState::default());
        st.set_liked("/m/b.flac", true);
        st.set_liked("/m/a.flac", true);
        st.set_liked("/m/b.flac", false);
        st.repeat = "stop".into();
        st.shuffle = "genre".into();
        st.volume = 0.5;
        st.eq.apply_preset("Jazz");
        save(&st, Some(&path)).unwrap();
        let again = load(Some(&path));
        assert_eq!(again.liked, vec!["/m/a.flac".to_string()]);
        assert_eq!((again.shuffle.as_str(), again.repeat.as_str(), again.volume), ("genre", "off", 0.5));
        assert_eq!(again.eq.preset, "Jazz");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
