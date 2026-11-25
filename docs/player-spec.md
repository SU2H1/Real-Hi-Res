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
  the app name in grey and Previous, Next and the heart are disabled (volume, EQ, Lyrics and Up Next stay usable);
  Play then plays the songs of the page shown.
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
  title and artist; double-click jumps to that song. A song can be dragged to another place (once the pointer has
  moved a few pixels; a line in the accent colour with a small circle at its left end shows the gap it will drop
  into, and the list scrolls when the pointer is near its top or bottom edge): `PlayQueue.move_upcoming(src, dst)`,
  where `dst` is its place after the move. A right click opens a menu with one entry, "Remove from Up Next":
  `PlayQueue.remove_upcoming(index)`. After either, the song that follows gaplessly is set again. A removed song is
  kept in `PlayQueue.removed` and stays out when the order is built again (another shuffle mode, repeat album) until
  a new list is played (`start()` empties it); moves are only kept until the order is built again.
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

`library_folders` lists the music folders. On start, the app shows the index saved in `library.sqlite` at once and
always refreshes it in a background thread (only new or changed files are probed; files that are gone are dropped),
updating the pages when it finishes; the same happens when the folders change. When `scan_library_on_start` ("Check
new songs when the app starts") is on, the refresh is followed by a quality scan of the library folders in the
Quality check page's scanner (saved results answer unchanged songs at once), without switching to that page. Before any folder is set, the Albums page
shows "Add your music folder in Settings to start" with a button "Choose a music folder…", which adds it and starts
the update. Covers come from `covers.cover_png()` (cached per album, loaded in a thread).

## Playback engine

- Decoding goes through FFmpeg (`probe.tool("ffmpeg")`, `external_env()`, no window on Windows), always to 32-bit
  float, stereo (mono files are duplicated, more channels are downmixed by FFmpeg `-ac 2`): `ffmpeg -nostdin -v
  error -ss <start> -i <file> -map 0:a:0 -vn [-af <filters>] -f f32le -ac 2 -ar <out rate> pipe:1`.
- The output opens at the song's sample rate when the device supports it ("Resampling: None"); otherwise at the
  device's default rate, and FFmpeg resamples with soxr (`aresample=resampler=soxr:precision=28`, or the default
  resampler when the FFmpeg build lacks soxr).
- Gapless (`gapless` on): the engine keeps about 2 s decoded ahead; when the current song has been decoded to its
  end it goes straight on decoding the next song of the queue (`PlayQueue.up_next(1)` when repeat is not song/stop;
  the same song again for repeat song) into the same buffer, so there is no silence between them, when that song gets
  the same output rate; otherwise the device is reopened at the next song's rate. The engine reports `Started` when a
  song's first sample is heard, `Ended(chained)` when its last one is (chained: the next song follows gaplessly) and
  `Failed`; on `Started` of a chained song the UI advances the queue with `next(auto=True)`.
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
  song paused at that position (it does not start playing by itself), with its album as the list it plays from.

## Lyrics while scanning

When `lyrics_while_scanning` is on, the Quality check scan (and "Check the library for fake files") passes every
analysed file to one lyrics worker (one `LrclibClient`, sequential requests, as LRCLIB asks), which follows the
Lyrics settings (`skip_reason()`, `lyrics_save_as`) and reports in the status line ("Lyrics: 12 saved, 3 not
found": " · Lyrics: N saved, M not found" at the end of the scanner's status line; saved = the outcome saved
anything, not found = NOT_FOUND). The scanner owns the worker (started on first use); Stop or a new scan stops it
after the current song. Lyrics written into a file keep its saved scan result (`ResultStore.retag()`).

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

## Additions (asked for on 2026-11-24, after the first version of this spec)

The user asked for a Library landing page with fourteen entries, playlists, dislikes, a play history with "Your
Stats", named equalizer presets that can be imported and exported, and custom themes; and that Settings → "Scan
library" opens the quality checker as it is. The core of all of it exists in both versions, checked against each
other (`tests/test_player_library.py`, `rust/crates/core/tests/player_additions.rs`): `musiclib` (composer tag,
`album_artists()`, `years()`, `composers()`, `folder_view()`, `album_sorted()`, the play history `add_play()`,
`plays()`, `recently_played()`, `clear_history()`), `stats` (`compute()`, `cards()`, `period_range()`,
`library_stats()`, `PERIODS`), `playerstate` (`disliked`, `set_disliked()`, playlists, `eq_presets`), `playqueue`
(`disliked`: left out of shuffled orders), `eq` (`export_json()`, `export_apo()`, `import_eq()`), `themes`
(`BUILTIN`, `load_custom()`, `save_custom()`, `resolve()`, `check_name()`, `copy_name()`, `export_theme()`,
`import_theme()`), and the `theme` setting, which is now free text holding a theme name.

### The Library page (the start page)

The app opens on **Library**: the title "Library", then the fourteen entries as a grid of tiles (icon, name, and a
count in grey: songs, folders, albums...), in this order: **All Songs**, **Folders**, **Albums**, **Artists**,
**Album Artists**, **Genres**, **Years**, **Composers**, **Playlists**, **Liked Songs**, **Disliked Songs**,
**Recently Played**, **Recently Added**, **Your Stats**. Under the tiles: a row of the most recently added albums
(cards) and, when there are plays this year, a small "Your Stats" banner with the minutes listened this year.

The sidebar lists the same: **Library** (the landing page) at the top, then the section "Library" with All Songs,
Folders, Albums, Artists, Album Artists, Genres, Years and Composers; the section "Playlists" with Liked Songs,
Disliked Songs and every playlist by name (and "New Playlist…"); the section "History" with Recently Played,
Recently Added and Your Stats; Settings at the bottom. The pages of the first version keep their behaviour ("Songs"
is now called All Songs).

- **Folders**: the library folders as the roots; a folder page shows its subfolders (folder icon, name, song count;
  a click opens it) above its songs (a song table in album order), with a breadcrumb ("Music › Jazz › 1999") whose
  parts are clickable; Play and Shuffle play the songs in the folder and below it.
- **Artists** (the track's artist), **Album Artists**, **Genres**, **Years** (newest first, "Unknown Year" last)
  and **Composers**: the names on the left with their counts, the songs or albums of the chosen one on the right
  (Artists and Album Artists show album cards; Genres, Years and Composers show song tables), each with Play and
  Shuffle.
- **Playlists**: a page with the playlists as cards (a 2×2 mosaic of the first four songs' covers, the name and the
  number of songs) and "New Playlist…". A playlist page: the mosaic, the name (Rename…, Delete… with a
  confirmation), "N songs, M minutes", Play, Shuffle, and the song table in the playlist's order, where the context
  menu adds "Remove from this playlist", "Move up" and "Move down". Every song's context menu (and the playback
  bar's "…" menu for the song playing) has "Add to playlist ▸" with the playlists and "New Playlist…". Playlist
  songs whose files are gone are shown in grey as "Not in the library" and skipped when played.
- **Disliked Songs**: the songs marked with the dislike button (a thumbs-down drawn as a vector icon, next to the
  heart in the playback bar and in the song's context menu: "Dislike" / "Remove dislike"). Liking a song removes
  its dislike and the other way round. Disliked songs are left out when shuffling (`PlayQueue.disliked`), unless
  you choose one yourself.
- **Recently Played**: the last 200 different songs played (`Library.recently_played()`), newest first, as a song
  table with a "Played" column ("Today 14:05", "Yesterday", or the date), and "Clear history…" (confirmation).
- **Recently Added**: as before.
- **Your Stats**: a period menu (`stats.PERIODS`, "This year" by default) and the summary as large coloured cards
  (`stats.cards()`: label small, big text bold, detail grey; each card in its own accent colour, like a yearly music
  review), then **Details**. The details show: the totals (minutes, plays, songs, artists, albums, genres, days with
  music, longest streak), bar charts of listening by hour of the day (24 bars), by weekday and by month, the top 50
  songs, artists, albums and genres as numbered lists with plays and minutes, and what the library holds
  (`stats.library_stats()`: songs, albums, artists, total length and size, formats, Hi-Res Lossless / Lossless /
  Lossy, sample rates and bit depths). Without plays in the period: "No listening yet in this period. Play some
  music and come back."

### Play history

A song is logged (`Library.add_play(track, started, seconds)`) when it stops being the current song (it ended, the
user skipped or stopped, chose another, or quit), with the seconds actually heard: the sum of the position's forward
steps between UI updates while playing, so seeking does not count; songs heard for less than a second are not
logged. `started` is the wall-clock time when it began. Settings → Library has "Clear the play history…".

### Equalizer presets

The preset menu of the equalizer lists the built-in presets, then "My presets" (`PlayerState.eq_presets`), and
"Custom" when the gains match none. A user preset holds the whole equalizer (graphic and parametric). Buttons:
**Save as preset…** (asks for a name; `check_preset_name()` errors are shown under the field), **Rename…**,
**Delete…**, **Import…** (a file dialog for .json and .txt: `eq.import_eq()`; the imported preset is applied and
saved under its name, a taken name getting " 2"; the notes are shown) and **Export…** (save as .json, Real-Hi-Res's
own format, or .txt, Equalizer APO / AutoEQ text: `export_json()` / `export_apo()`, by the extension chosen). The
name of the matching user preset (`matching_eq_preset()`) is shown in the menu.

### Themes

Settings → Appearance → Theme is a menu of `themes.names()`: "Match the system", the built-in themes (Light, Dark,
Midnight, Graphite, Forest, Ocean, Sepia, Rose) and the user's own. Under it the theme editor: **New theme…**
(a copy of the chosen theme, named by `copy_name()`), and for the user's own themes **Rename…**, **Delete…**, a
"Dark theme" switch and a colour button for each of the eight colours (`COLOR_KEYS`, labels from
`COLOR_LABELS`/`color_label()`), which change the app at once; **Import…** and **Export…** (.json). Built-in themes
cannot be edited; editing one offers to make a copy. The user's themes are saved in `themes.json`.

### Settings → Library

The buttons are: Update the library now; **Scan library** (opens the Quality check page, as in the first version,
with the library folders as its sources, and starts the scan); Download lyrics for the whole library…; Open the
quality checker; Clear the play history….

### Details settled while building both apps

- With one library folder, Folders opens it at once (the breadcrumb starts at that folder's name); the list of
  library folders appears only with several.
- The playlist mosaic shows the covers of up to four different albums of the playlist's songs; with fewer than four
  albums it shows the first album's cover full size.
- Your Stats → Details is a page of its own with a back link.
- New theme… makes the copy at once, named by `copy_name()`, and selects it; Rename… asks for a name.
- Choosing one of your own EQ presets keeps the equalizer on if it was on.
- When nothing plays, Previous, Next, the heart, the dislike button and "…" are disabled; Shuffle, Repeat, the
  volume, EQ, Lyrics and Up Next stay usable.
- Up Next can be edited (asked for on 2026-11-25): drag a song to reorder it, right-click → "Remove from Up Next"
  to take it out (see the Up Next panel above).
- A song counts in the play history from one second of listening; the seconds are the forward steps of the position
  while playing (the Python app ignores steps over 1.5 s, the Rust app over 2 s, between screen updates), so seeking
  never counts.
