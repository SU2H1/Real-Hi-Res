//! Read stream metadata with ffprobe and stream decoded PCM from ffmpeg
//! (the Rust counterpart of `realhires/probe.py`).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread::JoinHandle;

use crate::json::{self, Value};

/// Codecs whose output is lossy by design.
pub const LOSSY_CODECS: &[&str] = &[
    "mp3",
    "mp3float",
    "mp2",
    "mp2float",
    "mp1",
    "aac",
    "aac_latm",
    "aac_fixed",
    "vorbis",
    "opus",
    "wmav1",
    "wmav2",
    "wmavoice",
    "ac3",
    "eac3",
    "atrac1",
    "atrac3",
    "atrac3p",
    "atrac9",
    "musepack7",
    "musepack8",
    "speex",
    "amr_nb",
    "amr_wb",
    "gsm",
    "qdm2",
    "cook",
    "ra_144",
    "ra_288",
    "nellymoser",
    "dts",
];

/// Sample formats that do not carry a meaningful integer bit depth.
pub const FLOAT_SAMPLE_FMTS: &[&str] = &["flt", "fltp", "dbl", "dblp"];

/// The order of the normalised tags, as `normalize_tags()` builds them.
const TAG_ALIASES: &[(&str, &[&str])] = &[
    ("artist", &["artist", "iart", "author"]),
    ("album_artist", &["album_artist", "albumartist", "album artist", "aart", "band", "tpe2"]),
    ("album", &["album", "iprd"]),
    ("title", &["title", "inam"]),
    ("track", &["track", "tracknumber", "trck", "itrk"]),
    ("disc", &["disc", "discnumber", "tpos"]),
    ("date", &["date", "year", "icrd"]),
    ("genre", &["genre", "ignr"]),
];

pub type Tags = Vec<(String, String)>;

pub fn tag<'a>(tags: &'a Tags, key: &str) -> Option<&'a str> {
    tags.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

pub type Error = Box<dyn std::error::Error + Send + Sync>;

pub fn err(msg: impl Into<String>) -> Error {
    msg.into().into()
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn is_executable(p: &Path) -> bool {
    match std::fs::metadata(p) {
        Ok(m) if m.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                m.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        _ => false,
    }
}

/// Folders that may hold an FFmpeg shipped with a release package.
fn bundle_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(d) = std::env::var_os("REALHIRES_FFMPEG_DIR").filter(|d| !d.is_empty()) {
        out.push(PathBuf::from(d));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        out.push(dir.join("ffmpeg"));
        // macOS app bundle: Contents/MacOS/<exe> with FFmpeg in Contents/Resources/ffmpeg
        out.push(dir.join("..").join("Resources").join("ffmpeg"));
        out.push(dir);
    }
    out
}

/// Locate ffmpeg/ffprobe: a bundled copy first, then the system PATH.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let exe = exe_name(name);
    for d in bundle_dirs() {
        let cand = d.join(&exe);
        if is_executable(&cand) {
            return Some(cand);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(&exe)).find(|c| is_executable(c))
}

/// The path of a tool (looked up once), or its bare name when not found.
pub fn tool(name: &str) -> PathBuf {
    static CACHE: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().unwrap();
    map.entry(name.to_string()).or_insert_with(|| find_tool(name).unwrap_or_else(|| PathBuf::from(name))).clone()
}

/// A command for an external program, without a console window on Windows.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn require_tools() -> Result<(), String> {
    let missing: Vec<&str> = ["ffmpeg", "ffprobe"].into_iter().filter(|t| find_tool(t).is_none()).collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Required tool(s) not found: {}. Install FFmpeg (https://ffmpeg.org/download.html) or use a release \
             package, which bundles it.",
            missing.join(", ")
        ))
    }
}

#[derive(Clone, Debug, Default)]
pub struct AudioInfo {
    pub path: String,
    pub codec: String,
    pub container: String,
    pub sample_rate: u32,
    pub channels: u32,
    pub sample_fmt: String,
    pub claimed_bits: Option<u32>,
    pub is_float: bool,
    pub is_lossy: bool,
    pub duration: Option<f64>,
    pub bit_rate: Option<i64>,
    pub tags: Tags,
}

impl AudioInfo {
    /// True when the file advertises better-than-CD resolution.
    pub fn claims_hires(&self) -> bool {
        if self.is_lossy {
            return false;
        }
        let bits = self.claimed_bits.unwrap_or(16);
        self.sample_rate > 48000 || bits > 16
    }
}

/// Python's `int(value)` for ffprobe values, None unless positive.
fn int_or_none(v: Option<&Value>) -> Option<i64> {
    let n = match v? {
        Value::Int(i) => *i,
        Value::Float(f) if f.is_finite() => f.trunc() as i64,
        Value::Bool(b) => *b as i64,
        Value::Str(s) => s.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    (n > 0).then_some(n)
}

fn float_or_none(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Int(i) => Some(*i as f64),
        Value::Float(f) => Some(*f),
        Value::Str(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// Python's `x or y` for an optional float (0.0 counts as missing).
fn or_f(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match a {
        Some(x) if x != 0.0 => Some(x),
        _ => b,
    }
}

pub fn probe(path: &str) -> Result<AudioInfo, Error> {
    let out = command(tool("ffprobe"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name,sample_rate,channels,sample_fmt,bits_per_sample,bits_per_raw_sample,bit_rate,\
             duration:stream_tags:format=format_name,duration,bit_rate:format_tags",
            "-of",
            "json",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| err(format!("ffprobe failed: {e}")))?;
    if !out.status.success() {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(err(format!("ffprobe failed: {}", if msg.is_empty() { "unknown error" } else { &msg })));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let data = if text.trim().is_empty() { Value::Object(vec![]) } else { json::parse(&text)? };
    let streams = data.get("streams").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(s) = streams.first() else {
        return Err(err("no audio stream found"));
    };
    let empty = Value::Object(vec![]);
    let fmt = data.get("format").unwrap_or(&empty);
    let sget = |k: &str| s.get(k).and_then(Value::as_str).unwrap_or("").to_string();

    let codec = s.get("codec_name").and_then(Value::as_str).unwrap_or("unknown").to_string();
    let sample_fmt = sget("sample_fmt");
    let is_lossy = LOSSY_CODECS.contains(&codec.as_str());
    let mut is_float = FLOAT_SAMPLE_FMTS.contains(&sample_fmt.as_str()) || codec.starts_with("pcm_f");

    let mut bits = int_or_none(s.get("bits_per_raw_sample")).or_else(|| int_or_none(s.get("bits_per_sample")));
    if bits.is_none() {
        if sample_fmt.starts_with("s16") || sample_fmt.starts_with("u8") {
            bits = Some(if sample_fmt.starts_with("s16") { 16 } else { 8 });
        } else if sample_fmt.starts_with("s32") || sample_fmt.starts_with("flt") {
            bits = Some(32);
        } else if sample_fmt.starts_with("dbl") {
            bits = Some(64);
        }
    }
    if codec.starts_with("dsd_") {
        is_float = true; // DSD decodes to float PCM at fs/8
    }
    if is_lossy {
        bits = None;
    }
    Ok(AudioInfo {
        path: path.to_string(),
        codec,
        container: fmt.get("format_name").and_then(Value::as_str).unwrap_or("").to_string(),
        sample_rate: int_or_none(s.get("sample_rate")).unwrap_or(0) as u32,
        channels: int_or_none(s.get("channels")).unwrap_or(1) as u32,
        sample_fmt,
        claimed_bits: bits.map(|b| b as u32),
        is_float,
        is_lossy,
        duration: or_f(float_or_none(s.get("duration")), float_or_none(fmt.get("duration"))),
        bit_rate: int_or_none(s.get("bit_rate")).or_else(|| int_or_none(fmt.get("bit_rate"))),
        tags: normalize_tags(&[fmt.get("tags"), s.get("tags")]),
    })
}

/// Merge ffprobe format/stream tags into a small, case-insensitive set.
pub fn normalize_tags(sources: &[Option<&Value>]) -> Tags {
    let mut raw: Vec<(String, String)> = Vec::new();
    for src in sources.iter().flatten() {
        if let Some(items) = src.as_object() {
            for (k, v) in items {
                if let Some(v) = v.as_str() {
                    let (k, v) = (k.trim().to_lowercase(), v.trim());
                    if !v.is_empty() && !raw.iter().any(|(rk, _)| *rk == k) {
                        raw.push((k, v.to_string()));
                    }
                }
            }
        }
    }
    let mut out = Tags::new();
    for (key, names) in TAG_ALIASES {
        if let Some((_, v)) = names.iter().find_map(|n| raw.iter().find(|(rk, _)| rk == n)) {
            out.push((key.to_string(), v.clone()));
        }
    }
    out
}

/// Decoded audio from ffmpeg as left-justified int32 frames.
///
/// ffmpeg left-justifies every integer format into 32 bits (16-bit samples are
/// shifted left by 16, 24-bit by 8), so unused low bits stay exactly zero and
/// the bit-depth test can count them.  Float sources are scaled to the int32
/// range without dithering.
pub struct PcmReader {
    child: Option<Child>,
    stdout: Option<ChildStdout>,
    stderr: Option<JoinHandle<String>>,
    channels: usize,
    chunk_bytes: usize,
    pending: Vec<u8>,
    eof: bool,
    done: bool,
}

impl PcmReader {
    pub fn open(path: &str, channels: u32, chunk_frames: usize, max_seconds: Option<f64>) -> Result<Self, Error> {
        Self::open_args(path, channels, chunk_frames, max_seconds, &[])
    }

    /// `pre` goes before `-i` (e.g. `-sseof -1.5`).
    pub fn open_args(
        path: &str,
        channels: u32,
        chunk_frames: usize,
        max_seconds: Option<f64>,
        pre: &[String],
    ) -> Result<Self, Error> {
        let mut cmd = command(tool("ffmpeg"));
        cmd.args(["-v", "error", "-nostdin"]).args(pre).arg("-i").arg(path).args(["-map", "0:a:0", "-vn"]);
        if let Some(t) = max_seconds.filter(|t| *t > 0.0) {
            cmd.args(["-t", &format!("{t:.3}")]);
        }
        cmd.args(["-f", "s32le", "-acodec", "pcm_s32le", "-"]);
        let mut child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| err(format!("ffmpeg decode failed: {e}")))?;
        let stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let handle = std::thread::spawn(move || {
            let mut s = Vec::new();
            if let Some(e) = stderr.as_mut() {
                let _ = e.read_to_end(&mut s);
            }
            String::from_utf8_lossy(&s).into_owned()
        });
        let channels = channels.max(1) as usize;
        Ok(PcmReader {
            child: Some(child),
            stdout,
            stderr: Some(handle),
            channels,
            chunk_bytes: 4 * channels * chunk_frames.max(1),
            pending: Vec::new(),
            eof: false,
            done: false,
        })
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// The next block of interleaved samples (a whole number of frames), or
    /// None at the end.  A decoding error is reported at the end.
    pub fn next_block(&mut self) -> Result<Option<Vec<i32>>, Error> {
        if self.done {
            return Ok(None);
        }
        if self.eof {
            self.done = true;
            return self.finish().map(|_| None);
        }
        let frame_bytes = 4 * self.channels;
        let mut buf = std::mem::take(&mut self.pending);
        let start = buf.len();
        buf.resize(start + self.chunk_bytes, 0);
        let mut filled = start;
        let stdout = self.stdout.as_mut().expect("stdout");
        while filled < buf.len() {
            match stdout.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(err(format!("ffmpeg decode failed: {e}"))),
            }
        }
        buf.truncate(filled);
        self.eof = filled < start + self.chunk_bytes;
        let usable = buf.len() - buf.len() % frame_bytes;
        self.pending = buf.split_off(usable);
        if usable == 0 {
            self.done = true;
            return self.finish().map(|_| None);
        }
        Ok(Some(buf.chunks_exact(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()))
    }

    fn finish(&mut self) -> Result<(), Error> {
        self.stdout = None;
        let status = self.child.take().map(|mut c| c.wait());
        let msg = self.stderr.take().map(|h| h.join().unwrap_or_default()).unwrap_or_default();
        match status {
            Some(Ok(st)) if st.success() => Ok(()),
            Some(Ok(st)) => {
                let m = msg.trim();
                let code = st.code().map(|c| format!("exit code {c}")).unwrap_or_else(|| "killed".into());
                Err(err(format!("ffmpeg decode failed: {}", if m.is_empty() { code.as_str() } else { m })))
            }
            Some(Err(e)) => Err(err(format!("ffmpeg decode failed: {e}"))),
            None => Ok(()),
        }
    }

    /// Stop decoding early (the rest of the file is not needed).
    pub fn close(&mut self) {
        self.done = true;
        self.stdout = None;
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if let Some(h) = self.stderr.take() {
            let _ = h.join();
        }
    }
}

impl Drop for PcmReader {
    fn drop(&mut self) {
        self.close();
    }
}

/// Decode a whole file (or its first `max_seconds`) into one interleaved buffer.
pub fn read_all(path: &str, channels: u32, max_seconds: Option<f64>, pre: &[String]) -> Result<Vec<i32>, Error> {
    let mut r = PcmReader::open_args(path, channels, 1 << 16, max_seconds, pre)?;
    let mut out = Vec::new();
    while let Some(b) = r.next_block()? {
        out.extend_from_slice(&b);
    }
    Ok(out)
}
