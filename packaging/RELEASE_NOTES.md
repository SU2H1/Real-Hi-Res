## What's new in 2.0.0

**Real-Hi-Res is now Universal Music Player, a music player.** It has moved to this repository, [SU2H1/Universal-Music-Player](https://github.com/SU2H1/Universal-Music-Player), with its whole history, and is licensed under the GNU General Public License version 3. The quality checker keeps the name Real-Hi-Res, and your saved results, settings and the command-line tools (`realhires-cli`, Rust `realhires`) carry on unchanged; [Real-Hi-Res](https://github.com/SU2H1/Real-Hi-Res) itself stays at 1.12.1.

**A music player.** The desktop app opens on your **Library**, in a window that looks and works much like Apple Music, and the fake-hi-res checker you know lives on inside it. Both versions, Python and Rust, are the same player.

- **The Library page** leads to All Songs, Folders, Albums, Artists, Album Artists, Genres, Years, Composers, Playlists, Liked Songs, Disliked Songs, Recently Played, Recently Added and **Your Stats**, each with a count. Choose your music folder once (Settings → Library); the tags are read in the background and kept, so the next start is instant.
- **Playback** through FFmpeg at each song's own sample rate where the sound device allows it (otherwise with the SoX resampler, and the Playback info says so), **gapless** from song to song, with a remembered volume, mode and last song.
- **The playback bar**: Shuffle (off, this list, the whole library, same genre, same artist or same album), Previous, Play/Pause, Next, Repeat (off, song, album, stop after this song), a seek bar, a quality badge (**Hi-Res Lossless**, **Lossless** or the codec) that opens the **Playback info** (track, decoder, output, resampling, volume, equalizer and the song's Real-Hi-Res verdict), Like, **Dislike** (disliked songs are left out of shuffles), "…" (add to a playlist and more), Lyrics, Up Next, the equalizer and the volume.
- **Now Playing** shows the album cover on the left and the **synced lyrics** on the right, following the music line by line (a click on a line jumps there); without lyrics the cover is centred, with a **Get lyrics** button.
- **Playlists** of your own: make, rename, delete, reorder, and add songs from any song's menu.
- **Your Stats**: what you played this year (or the last 30 days, last year, all time) as colourful cards in the style of a yearly music review, minutes, top song, artist, album and genre, your listening hour and day, your longest streak, and **Details** with charts by hour, weekday and month, your top 50 songs, artists, albums and genres, and what your library holds.
- **The equalizer**: a 10-band graphic equalizer with 17 presets and a parametric equalizer with up to 10 bands, a response curve and "prevent clipping". **Name your own presets**, and **import and export** them as Real-Hi-Res `.json` files or as Equalizer APO / AutoEQ `.txt` files (so AutoEQ's headphone corrections can be loaded).
- **Themes**: Match the system, eight built-in themes (Light, Dark, Midnight, Graphite, Forest, Ocean, Sepia, Rose), and **your own**, with a colour editor, import and export. **Theme files** put into the themes folder (Settings → Appearance → Open themes folder) appear in the theme menu as they are, and the Rust version imports a theme file dropped onto its window. On a Mac the app uses SF Pro (SF Pro Display Bold for headings); Windows and Linux use the similar Inter typeface, now built in; any installed font can be chosen.
- **Edit song information**: Edit info… (a song's menu) and Edit album info… (an album page) change the title, artist, album, album artist, genre, release date, track, disc and composer of one song or a whole album. FLAC, MP3, Ogg Vorbis and Opus tags are written directly, other formats are copied by FFmpeg without re-encoding and checked sample for sample; the audio never changes, and the library and the saved check results follow.
- **Lyrics inside the file**: Settings → Lyrics → Save lyrics as can write synced lyrics into FLAC (`LYRICS`) and MP3 (`SYLT`) files, or plain lyrics, or both an `.lrc` file and the tags, as LRCGET does; only the tags are rewritten, and the file's saved check result stays valid. **Download lyrics while scanning** fetches them as the scan goes.
- **Everything else is in Settings**: Settings → Library → **Scan library** opens the quality checker of earlier versions with your library folders and checks them for fake hi-res and fake lossless files; **Check new songs when the app starts** does it in the background. Fix, the lyrics window, saved results, the album and artist views and all the settings are where they were, on the **Quality check** page.

The command-line tools are unchanged, apart from reading the composer tag.

**New requirements.** The Python packages bundle [miniaudio](https://github.com/irmen/pyminiaudio) for the sound output. The Linux packages use the system's ALSA library for sound, which every Linux desktop has (it also reaches PulseAudio and PipeWire). Running from source needs `pip install miniaudio` (or `pip install .`), and building the Rust version needs Rust 1.95 or newer and, on Linux, the ALSA headers.

## Download

Pick the package for your computer, unpack it and run it. Python, NumPy and FFmpeg are bundled, so nothing else has to be installed.

| System | Download | Start the app |
| --- | --- | --- |
| Windows 10/11, 64-bit | `UniversalMusicPlayer-<version>-windows-x64.zip` | `UniversalMusicPlayer\UniversalMusicPlayer.exe` |
| macOS 12+ on Apple Silicon (M1 and later) | `UniversalMusicPlayer-<version>-macos-arm64.dmg` | drag `UniversalMusicPlayer` to Applications |
| macOS 10.15+ on Intel | `UniversalMusicPlayer-<version>-macos-x86_64.dmg` | drag `UniversalMusicPlayer` to Applications |
| Linux x86-64 | `UniversalMusicPlayer-<version>-linux-x64.tar.gz` | `UniversalMusicPlayer/UniversalMusicPlayer` (optional: `UniversalMusicPlayer/install.sh` adds a menu entry) |
| Linux ARM64 (e.g. Raspberry Pi 4/5 with 64-bit OS) | `UniversalMusicPlayer-<version>-linux-arm64.tar.gz` | `UniversalMusicPlayer/UniversalMusicPlayer` (optional: `UniversalMusicPlayer/install.sh`) |

**Rust version.** The same music player and command-line tool, written in Rust, with FFmpeg bundled; it shares your library, likes, playlists, play history, themes and saved results with the version above:

| System | Download | Start the app |
| --- | --- | --- |
| Windows 10/11, 64-bit | `UniversalMusicPlayer-Rust-<version>-windows-x64.zip` | `UniversalMusicPlayer-Rust\universal-music-player.exe` |
| macOS 12+ on Apple Silicon | `UniversalMusicPlayer-Rust-<version>-macos-arm64.dmg` | drag `UniversalMusicPlayer-Rust` to Applications |
| macOS 10.15+ on Intel | `UniversalMusicPlayer-Rust-<version>-macos-x86_64.dmg` | drag `UniversalMusicPlayer-Rust` to Applications |
| Linux x86-64 | `UniversalMusicPlayer-Rust-<version>-linux-x64.tar.gz` | `UniversalMusicPlayer-Rust/universal-music-player` (optional: `UniversalMusicPlayer-Rust/install.sh`) |
| Linux ARM64 | `UniversalMusicPlayer-Rust-<version>-linux-arm64.tar.gz` | `UniversalMusicPlayer-Rust/universal-music-player` (optional: `UniversalMusicPlayer-Rust/install.sh`) |

Its command-line tool is `realhires` (`realhires.exe`) next to the app, with the same options as `realhires-cli`.

The `.zip` files for macOS contain the same app as the `.dmg` files. The Linux packages need glibc 2.28 or newer (any distribution from 2018 onwards), and the Rust version's app also needs OpenGL, which every desktop has. Every package also contains the command-line tool. On Windows, keep the `.pkg` files and the `_internal` folder next to the `.exe` files of the Python version, and the `ffmpeg` folder next to those of the Rust version.

**First launch.** The apps are not code-signed. On Windows, SmartScreen may ask you to confirm with **More info → Run anyway**. On macOS 15 and later, open **System Settings → Privacy & Security** and click **Open Anyway**. On older macOS, right-click the app and choose **Open**, or run `xattr -dr com.apple.quarantine /Applications/UniversalMusicPlayer.app` (or `/Applications/UniversalMusicPlayer-Rust.app`) once in Terminal.

## Credits

Synced lyrics come from [LRCLIB](https://lrclib.net) and the people who contribute to it, and the lyrics feature is modelled on LRCGET by tranxuanthang ([github.com/tranxuanthang/lrcget](https://github.com/tranxuanthang/lrcget)), who also makes LRCLIB. Both are MIT-licensed, and Real-Hi-Res's own lyrics code contains no code from either. Both apps credit them in the lyrics window and in the About box (Settings → About Universal Music Player). Thank you to them.

On Windows and Linux the apps use the [Inter](https://rsms.me/inter/) typeface by The Inter Project Authors (SIL Open Font License 1.1; its licence is in every package). The Python app plays sound through [miniaudio](https://github.com/irmen/pyminiaudio) (Irmen de Jong and David Reid, MIT License), the Rust app through [cpal](https://github.com/RustAudio/cpal) (Apache License 2.0).

The bundled FFmpeg executables are licensed separately (LGPL on Windows and Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
