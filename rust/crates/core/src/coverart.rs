//! Covers from MusicBrainz and the Cover Art Archive (2.5, `docs/player-spec.md`,
//! "2.5", item 9): the port of Python's `realhires/coverart.py`.
//!
//! Search Cover Online → MusicBrainz… looks an album up on MusicBrainz (the
//! existing client: one request a second, the app's User-Agent) and shows the
//! front covers of up to `MAX_RELEASES` matching releases from the Cover Art
//! Archive (<https://coverartarchive.org>): `/release/<mbid>/front-250` for the
//! grid and `/release/<mbid>/front` for the one the user chose.
//!
//! The archive gets the same care as MusicBrainz: `CoverArtClient` sends one
//! download at a time (across the app, `GATE`), names the app in the User-Agent,
//! follows the archive's redirects, reads at most `MAX_BYTES` and never tries a
//! download twice; when the archive asks to slow down (HTTP 429/503) nothing more
//! is asked for (`CoverArtError::Busy`).  Only the album's artist and title are
//! sent (to MusicBrainz), and a release's id (to the archive); never a path.
//! The address is `DEFAULT_SERVER`, or `$REALHIRES_COVERART` for tests, which
//! must never contact the real archive.
//!
//! Written for Universal Music Player against the archive's published API; it
//! contains no code from MusicBrainz or Picard.  Requests and texts are checked
//! against `tests/python_jacket.json`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::json::Value;
use crate::lyrics::{self, Transport, TransportError};
use crate::musicbrainz::{self, MbError, MusicBrainzClient, Release};
use crate::tagedit::clean_value;

pub const DEFAULT_SERVER: &str = "https://coverartarchive.org";
/// Tests point it at a fake archive on 127.0.0.1.
pub const SERVER_ENV: &str = "REALHIRES_COVERART";
/// A picture larger than this is not taken.
pub const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Releases asked of MusicBrainz, and covers shown.
pub const MAX_RELEASES: usize = 12;
/// The grid's pictures: `/front-250`.
pub const THUMB_PX: u32 = 250;
pub const TIMEOUT_S: f64 = 30.0;

pub const CREDIT: &str = "Covers come from the Cover Art Archive (coverartarchive.org), found through MusicBrainz \
                          (musicbrainz.org). The pictures belong to their owners.";
pub const ABOUT_CREDIT: &str = "Cover search: MusicBrainz and the Cover Art Archive (coverartarchive.org).";

#[derive(Debug, Clone, PartialEq)]
pub enum CoverArtError {
    /// A picture could not be had (the message is a full sentence).
    Failed(String),
    /// The archive asked to slow down: nothing more is asked for now.
    Busy(String),
    /// The window was closed.
    Stopped,
}

impl std::fmt::Display for CoverArtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoverArtError::Failed(m) | CoverArtError::Busy(m) => f.write_str(m),
            CoverArtError::Stopped => f.write_str("Stopped."),
        }
    }
}

pub fn normalize_server(address: &str) -> String {
    let mut s = address.trim().trim_end_matches('/').to_string();
    if s.is_empty() {
        s = DEFAULT_SERVER.into();
    }
    if !s.contains("://") {
        s = format!("https://{s}");
    }
    s
}

/// The archive's address: `$REALHIRES_COVERART` (tests), else `DEFAULT_SERVER`.
pub fn server() -> String {
    normalize_server(&std::env::var(SERVER_ENV).unwrap_or_default())
}

/// Where a release's front cover is: the 250-pixel one for the grid, or as it
/// was uploaded.  `base`: the archive's address (None: `server()`).
pub fn front_url(mbid: &str, thumb: bool, base: Option<&str>) -> String {
    let base = base.map(normalize_server).unwrap_or_else(server);
    let what = if thumb { format!("front-{THUMB_PX}") } else { "front".to_string() };
    format!("{base}/release/{}/{what}", musicbrainz::quote_segment(mbid))
}

/// (release, artist) that MusicBrainz is asked for: the album and its artist (the
/// album artist, else the artist); a song without an album: its title and artist.
pub fn search_terms(album_artist: &str, artist: &str, album: &str, title: &str) -> (String, String) {
    let or = |a: &str, b: &str| {
        let a = clean_value(a);
        if a.is_empty() {
            clean_value(b)
        } else {
            a
        }
    };
    let album = clean_value(album);
    if !album.is_empty() {
        (album, or(album_artist, artist))
    } else {
        (clean_value(title), or(artist, album_artist))
    }
}

/// Up to `MAX_RELEASES` releases called `album` (by `artist`), as MusicBrainz
/// ranks them, each once.
pub fn find_releases(client: &mut MusicBrainzClient, album: &str, artist: &str) -> Result<Vec<Release>, MbError> {
    let params = [("query".to_string(), musicbrainz::release_query(album, artist)), ("limit".to_string(), MAX_RELEASES.to_string())];
    let data = client.request("release", &params)?;
    let mut out: Vec<Release> = vec![];
    let found = data.as_ref().and_then(|d| d.get("releases")).and_then(Value::as_array);
    for r in found.into_iter().flatten().filter(|x| matches!(x, Value::Object(_))) {
        let r = musicbrainz::parse_release(r);
        if !r.id.is_empty() && !out.iter().any(|o| o.id == r.id) {
            out.push(r);
        }
    }
    out.truncate(MAX_RELEASES);
    Ok(out)
}

/// The grey line under a cover: the year, the country and the format
/// ("2001 · JP · CD").
pub fn release_facts(r: &Release) -> String {
    let year: String = r.date.chars().take(4).collect();
    [year.as_str(), r.country.as_str(), r.formats.as_str()].iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}

pub fn megabytes(n: usize) -> String {
    format!("{} MB", n / (1024 * 1024))
}

/// One download at a time across the app, whatever window asks.
static GATE: Mutex<()> = Mutex::new(());

/// Front covers from the Cover Art Archive, one download at a time, each tried once.
pub struct CoverArtClient {
    pub server: String,
    stop: Option<Arc<AtomicBool>>,
    transport: Transport,
    pub timeout: f64,
    pub max_bytes: usize,
    pub requests: usize,
}

impl CoverArtClient {
    /// `base`: the archive's address (None: `server()`).
    pub fn new(base: Option<&str>) -> Self {
        CoverArtClient {
            server: base.map(normalize_server).unwrap_or_else(server),
            stop: None,
            // one byte more than is taken, so a picture that is too large is seen to be
            transport: lyrics::http_transport_limited(MAX_BYTES as u64 + 1),
            timeout: TIMEOUT_S,
            max_bytes: MAX_BYTES,
            requests: 0,
        }
    }

    pub fn with_stop(mut self, stop: Arc<AtomicBool>) -> Self {
        self.stop = Some(stop);
        self
    }

    /// Another way to download (the tests): it is given the address, the headers
    /// and the time allowed, and answers (status, headers, body).
    pub fn with_transport(mut self, t: Transport) -> Self {
        self.transport = t;
        self
    }

    pub fn with_max_bytes(mut self, n: usize) -> Self {
        self.max_bytes = n;
        if n < MAX_BYTES {
            self.transport = lyrics::http_transport_limited(n as u64 + 1);
        }
        self
    }

    pub fn host(&self) -> String {
        let n = lyrics::netloc(&self.server);
        if n.is_empty() {
            self.server.clone()
        } else {
            n.to_string()
        }
    }

    /// The front cover of a release (JPEG, PNG...), or None when the archive has none.
    pub fn front(&mut self, mbid: &str, thumb: bool) -> Result<Option<Vec<u8>>, CoverArtError> {
        let url = front_url(mbid, thumb, Some(&self.server));
        let ua = musicbrainz::user_agent();
        let headers = [("User-Agent", ua.as_str()), ("Accept", "image/*")];
        let host = self.host();
        let answer = {
            let _one = GATE.lock().unwrap_or_else(|e| e.into_inner());
            if self.stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed)) {
                return Err(CoverArtError::Stopped);
            }
            self.requests += 1;
            (self.transport)(&url, &headers, self.timeout)
        };
        let (status, _, body) = match answer {
            Ok(a) => a,
            Err(TransportError::Invalid(e)) => {
                return Err(CoverArtError::Failed(format!(
                    "The Cover Art Archive address {} is not valid: {e}.",
                    lyrics::repr(&self.server)
                )))
            }
            Err(TransportError::Certificate(e)) => {
                return Err(CoverArtError::Failed(format!(
                    "Could not connect to {host} securely: the certificate of the server could not be verified ({e})."
                )))
            }
            Err(TransportError::Network(e)) => return Err(CoverArtError::Failed(format!("Could not reach {host}: {e}."))),
        };
        if status == 404 {
            return Ok(None);
        }
        if status == 429 || status == 503 {
            return Err(CoverArtError::Busy(format!("{host} is busy and asked to slow down. Try again in a few minutes.")));
        }
        if status != 200 {
            return Err(CoverArtError::Failed(format!("{host} could not send the picture (HTTP {status}).")));
        }
        if body.len() > self.max_bytes {
            return Err(CoverArtError::Failed(format!(
                "The picture is larger than {} and was not downloaded.",
                megabytes(self.max_bytes)
            )));
        }
        if crate::covers::image_format(&body).is_empty() {
            return Err(CoverArtError::Failed(format!("{host} sent something that is not a picture.")));
        }
        Ok(Some(body))
    }
}
