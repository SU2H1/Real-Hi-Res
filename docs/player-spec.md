# Real-Hi-Res 2.0 music player: shared specification

Both desktop apps (Python/Tkinter `realhires/gui.py` + new modules, and Rust/egui `rust/crates/gui`) implement this
specification, so they behave the same. The maths and data live in the shared core modules, which both versions
already have and which are checked against each other: `eq` (equalizer), `playqueue` (shuffle/repeat queue),
`playerstate` (`player.json`: likes, volume, modes, EQ, last song), `musiclib` (`library.sqlite`: the tag index of
the library folders), `embed` (lyrics inside MP3/FLAC files) and `lyrics` (LRCLIB, with `lyrics_save_as`).

## What the user asked for (2026-11-24)

The app becomes a music player that looks similar to Apple Music, with SF Pro Display Bold for headings (SF Pro on
macOS from the system; Windows and Linux use a similar font, Inter, bundled under the SIL Open Font License; the user
can choose another font in Settings). Every function the app has today (scan for fake files, Fix, lyrics download,
saved results, all settings) is reached from Settings. The Now Playing view shows the album picture on the left and
the lyrics on the right; without lyrics the cover is centred. Songs can be liked. The playback info is easy to see,
and clicking it shows the track info and the decoder. Shuffle offers: Off, Songs in this list, All songs in the
library, Same genre, Same artist, Same album. Repeat offers: Off, Repeat song, Repeat album, Stop after this song.
There is a graphic and a parametric equalizer. Lyrics can be downloaded as the scan goes, and saved as an .lrc file,
inside the audio file, both, or as plain lyrics inside the file (`lyrics_save_as`, already in the core).

## Window

- **Sidebar** (left, about 220 px, slightly darker than the content): a Search box at the top; the heading
  "Library" with Recently Added, Artists, Albums, Songs, Genres; the heading "Playlists" with Liked Songs; at the
  bottom Settings, and a one-line status while the library index is being updated ("Updating the library: 120 of
  500 songs").
- **Playback bar** (top, about 64 px): left the buttons Shuffle, Previous, Play/Pause, Next, Repeat; in the middle
  the "LCD": a rounded panel with a 44 px cover, the title (bold), "Artist — Album" (grey), a thin seek bar
  (click or drag to seek) with the elapsed time on the left and the remaining time ("-1:23") on the right, and a
  quality badge ("Hi-Res Lossless", "Lossless", or the codec and bit rate, `musiclib.quality()`); right: the heart
  (like the current song), Lyrics (toggles the Now Playing view), Up Next (toggles the queue panel), EQ (opens the
  equalizer window), and the volume: a speaker icon (click mutes) and a slider. When nothing plays, the LCD shows
  the app name in grey and every control except Play is disabled; Play then plays the songs of the page shown.
- **Icons** are drawn as vector shapes (Tk Canvas, egui painter), never emoji or rare Unicode symbols, which several
  Linux fonts cannot show. A mode that is on (shuffle not off, repeat not off) is drawn in the accent colour; repeat
  song adds a small "1", stop after this song draws a small square.
- **Shuffle button**: a click opens a menu with the six modes as radio items (labels from
  `playqueue.SHUFFLE_LABELS` / `shuffle_label()`); choosing one calls `PlayQueue.set_shuffle()` and saves it in the
  player state. **Repeat button**: the same with the four repeat modes (`REPEAT_LABELS` / `repeat_label()`). The
  tooltip names the current mode. "Stop after this song" is never saved (the core already drops it on load).
- **Pages** in the content area:
  - Recently Added: album cards, newest first (the newest `mtime_ns` of an album's tracks).
  - Albums: album cards in `musiclib.albums()` order. A card is the square cover (rounded corners), the album title
    (bold) and the artist (grey) below it. Clicking a card opens the album page.
  - Album page: the cover (about 220 px) on the left; on the right the title (large, bold), the artist (accent
    colour), "Genre · Year" (grey), the buttons Play and Shuffle (accent filled), then the track list: number, title,
    a heart for liked songs, the duration, and at the end "N songs, M minutes". Double-click (or Enter) plays from
    that song with the album as the list.
  - Artists: the artists (`musiclib.artists()`) as a list; choosing one shows its albums as cards, with Play and
    Shuffle for all its songs.
  - Songs: a table (Title, Artist, Album, Genre, Time, Quality, and a heart column); a click on a column heading
    sorts by it; double-click plays from that song with the shown rows as the list.
  - Genres: the genres (`musiclib.genres()`); choosing one shows its songs as the table above.
  - Liked Songs: the liked songs as the table above (`PlayerState.liked`, library order).
  - Search: typing in the sidebar's Search box (after a 200 ms pause) shows the albums and songs whose title, artist,
    album or genre contains the text (case-folded); clearing it returns to the page before.
  - Now Playing (the Lyrics button, or a click on the LCD's cover): the cover on the left half (as large as fits, at
    most 480 px) and the lyrics on the right. Synced lyrics (`embed.lrc_entries()`): every line, the current one
    (the last line whose time is not after the position) bold in the text colour, the others grey; the view scrolls
    smoothly to keep the current line about a third from the top unless the user scrolled in the last 3 s; a click
    on a line seeks there. Plain lyrics: the text, scrollable. No lyrics: the cover centred with the title and artist
    under it, and a button "Get lyrics" that downloads them for this song (the lyrics window, one song). The lyrics
    are found with `lyrics.read_lyrics_file()` (an .lrc/.txt next to the song, else the lyrics inside the file).
  - Settings (see below) and Quality check (the 1.x scanner UI, unchanged, with all its functions).
- **Up Next panel** (right, about 300 px, toggled): "Playing from <list name>", then `PlayQueue.up_next(50)` with
  title and artist; double-click jumps to that song.
- **Playback info window** (a click on the quality badge): three sections, each a two-column list, and Copy.
  "Track": Title, Artist, Album, Album artist, Genre, Year, Track ("3 of 12" when known), Disc, Duration, File (full
  path), Size. "Decoder": Format (codec upper-case, e.g. FLAC, with "lossless"/"lossy"), Sample rate ("96 kHz"),
  Bit depth ("24-bit", "32-bit float", or "—" for lossy), Channels ("Stereo", "Mono", "6 channels"), Bit rate
  ("2,304 kbps"), Decoder ("FFmpeg", plus its version line when known). "Output": Device (name), Output ("96 kHz,
  32-bit float, stereo"), Resampling ("None: the device runs at the song's rate" or "From 96 kHz to 48 kHz (soxr)"),
  Volume ("−6.0 dB" or "Muted"), Equalizer ("Off", "Graphic: Rock", "Parametric: 3 bands", plus " (pre-amp −4.5
  dB)" when not zero), Gapless ("On"/"Off"). A fourth section "Quality check" shows the saved Real-Hi-Res verdict and
  summary of the file when the results store has one ("Not checked yet" otherwise).
- **Equalizer window**: "Equalizer on" checkbox (`enabled`), "Prevent clipping" (`prevent_clipping`, automatic
  headroom from `eq.preamp_db()`), two tabs Graphic and Parametric (each with its own `enabled`). Above the tabs the
  combined response curve (`eq.response()`, 20 Hz–20 kHz log axis, ±12 dB). Graphic: a preset menu (`PRESETS` plus
  "Custom", which is shown when the gains match no preset, `preset_name()`), a pre-amp slider and ten vertical
  sliders (−12…+12 dB, labels `GRAPHIC_LABELS`), double-click on a slider resets it to 0. Parametric: a pre-amp slider
  and one row per band: on, type (`BAND_TYPE_LABELS`), frequency (20–20000 Hz), gain (ignored and disabled for the
  types without gain), Q (0.1–10), and Remove; "Add band" until `MAX_PARAMETRIC_BANDS`. Changes are heard at once
  (Rust: the biquads are recomputed in the audio thread; Python: the decoder restarts at the current position with the
  new `eq.ffmpeg_filter()`) and saved in the player state when the window closes or the app quits.
- **Likes**: the heart in the playback bar and the heart column of every song table toggle `PlayerState.set_liked()`;
  the state is saved at once.

## Settings page

The Settings page replaces the Settings window of 1.x (the window stays reachable from the Quality check page). It
has one tab (or section list on the left) per `settings.GROUPS` entry, built from `Settings.field_info()` as before,
and adds actions:

- Library: the folder list (`library_folders`) with Add folder… and Remove; "Update the library now" (re-reads the
  tags with `Library.refresh(library_files(cfg))`); "Check the library for fake files" (opens the Quality check page
  with the library folders as its sources and starts a scan); "Download lyrics for the whole library…" (the lyrics
  window with every library song); "Open the quality checker" (the Quality check page as it was).
- Playback: `output_device` (a menu of the system's output devices, "" = the system default), `gapless`, and
  "Equalizer…".
- Appearance: `font_family` (a menu of the installed font families, "" = "SF Pro (macOS) / Inter"), `theme`.
- Lyrics: as before, plus `lyrics_save_as` and `lyrics_while_scanning`.
- Saved results: as before, with "Delete all saved results…".

Every change is saved at once (as in 1.x); fonts and theme apply at once.

## Library index

`library_folders` lists the music folders. On start, the app shows the index saved in `library.sqlite` at once, and
when `scan_library_on_start` is on it refreshes it in a background thread (only new or changed files are probed;
files that are gone are dropped), updating the pages when it finishes. Before any folder is set, the Albums page
shows "Add your music folder in Settings to start" with a button "Choose a music folder…", which adds it and starts
the update. Covers come from `covers.cover_png()` (cached per album, loaded in a thread).

## Playback engine

- Decoding goes through FFmpeg (`probe.tool("ffmpeg")`, `external_env()`, no window on Windows), always to 32-bit
  float, stereo (mono files are duplicated, more channels are downmixed by FFmpeg `-ac 2`): `ffmpeg -nostdin -v
  error -ss <start> -i <file> -map 0:a:0 -vn [-af <filters>] -f f32le -ac 2 -ar <out rate> pipe:1`.
- The output opens at the song's sample rate when the device supports it ("Resampling: None"); otherwise at the
  device's default rate, and FFmpeg resamples with soxr (`aresample=resampler=soxr:precision=28`, or the default
  resampler when the FFmpeg build lacks soxr).
- Gapless (`gapless` on): about 10 s before the end of a song the engine starts decoding the next song of the queue
  (`PlayQueue.up_next(1)` when repeat is not song/stop; the same song again for repeat song) into the same buffer,
  so there is no silence between them, when its output rate is the same; otherwise the device is reopened at the next
  song's rate. The UI learns that the next song started when its first sample is played, and then advances the
  queue with `next(auto=True)`.
- Volume: gain = volume² (volume 0…1 from the slider), 0 when muted; applied in the output callback, after the EQ.
  The Playback info shows it as dB (20·log10(gain)).
- EQ: when `enabled` and the active stages are not all identity, the stages of `eq.stages(state, out_rate)` run on
  the float samples before the volume, with the pre-amp `eq.preamp_db()` (0 when prevent clipping is off). Rust runs
  them in the audio callback (`EqProcessor`); Python lets FFmpeg run them (`eq.ffmpeg_filter()`).
- Position: the frames played since the song started (not decoded), so the seek bar and the lyrics follow what is
  heard. Seeking restarts FFmpeg at the new position and drops the buffer.
- Errors (no output device, unreadable file) show in the LCD's second line for that song, and playback moves to the
  next song after 2 s, except when every song failed.
- On quit the player state saves the song and position (`last_path`, `last_position`); on start the LCD shows that
  song paused at that position (it does not start playing by itself).

## Lyrics while scanning

When `lyrics_while_scanning` is on, the Quality check scan (and "Check the library for fake files") passes every
analysed file to one lyrics worker (one `LrclibClient`, sequential requests, as LRCLIB asks), which follows the
Lyrics settings (`skip_reason()`, `lyrics_save_as`) and reports in the status line ("Lyrics: 12 saved, 3 not
found"). Stopping the scan stops the worker after the current song.

## Fonts and theme

- macOS: the system font (`/System/Library/Fonts/SFNS.ttf`, the variable SF Pro) at weight 700 for headings ("SF Pro
  Display Bold") and 400 for text; Windows and Linux: the bundled Inter (`realhires/assets/fonts/InterVariable.ttf`,
  with `LICENSE-Inter.txt`), 700 and 400. Tk uses the family names ("SF Pro Display"/".AppleSystemUIFont" on macOS,
  "Inter" after loading the bundled file where Tk can, else the platform's UI font) with the bold weight.
- `font_family` set: that family for everything, bold for headings.
- Colours (light / dark): background #FFFFFF / #1E1E1E, sidebar #F5F5F7 / #262628, text #1D1D1F / #F5F5F7, secondary
  text #6E6E73 / #A1A1A6, separators #D2D2D7 / #3A3A3C, accent #FA2D48 (both), the LCD panel #F2F2F4 / #2C2C2E.
  `theme` "Match the system" follows the system's dark mode where it can be read (egui does; Tk reads the macOS and
  Windows setting and otherwise uses light).

## Quality check page

The 1.x app unchanged (sources, list, dashboard, summary, Fix, lyrics, find bar, card views, export), shown in the
content area with the sidebar beside it, so every existing test keeps working. Its scan also feeds the lyrics worker
when `lyrics_while_scanning` is on.
