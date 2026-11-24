## What's new in 2.2.0

**A cleaner window.** The menu bar is gone from both apps. Quality Check is always in the sidebar, the About box is on the Settings page, and the Quality Check page now looks like the rest of the player, in your theme: a page title with the scan's status under it, an accent-coloured Scan button, verdict chips with counts, and a **…** (More) button that holds what the old menus held. Every keyboard shortcut works as before. The **mouse's back and forward buttons** (and Alt+← / Alt+→, Cmd+[ / Cmd+] on a Mac) move between pages, as in a browser. The app has a new icon.

**Bit-perfect playback** (Settings → Playback, off by default) sends each song to your sound device exactly as it is in the file: at its own sample rate, without the equalizer, the volume control or resampling, and with the device to the app alone where the system allows it (WASAPI exclusive mode on Windows, the ALSA hardware device on Linux; on macOS the device is switched to the song's rate). Set the volume on your amplifier or DAC. The Playback info says whether a song really plays bit-perfect, and if not, why. The tests check that 16-bit and 24-bit songs reach the output sample for sample; the exclusive device access could only be built and checked on CI, so please tell us how it works with your DAC.

**Now Playing**
- A switch above the lyrics: **Lyrics**, **Up Next** (the queue) and **Details** (the song's information, format, comment and Quality Check verdict).
- The lyrics say whether they are synced or plain text and where they come from; **A− / A+** change their size (remembered); after you scroll synced lyrics, **Back to the current line** returns to the music.
- The Lyrics button hides the lyrics and leaves you on the page, with the cover centred; the cover, title and artist form one centred block.
- The album name opens the album and each artist name opens that artist (also in the playback bar). A click on the cover opens the new **Cover** window.
- **Get lyrics** now downloads at once (it used to open a window that waited for a second click), from Now Playing and from every song's menu.
- Lyrics saved by **LRCGET** now show synced: synced lyrics inside a file used to lose to a plain `.txt` next to it, and LRC text kept as plain lyrics now counts as synced.

**Your library**
- **Play Next** and **Add to Queue** in every song's menu.
- **Acquire Metadata…** in the right-click menu of songs and of albums fills in their information from MusicBrainz; albums' covers have a right-click menu everywhere now (Play, Shuffle, Play Next, Add to Queue, Acquire Metadata…, Album Properties…).
- Artists, Album Artists, Genres, Years and Composers are **card grids** like Albums.
- **Several artists in one field** ("Mariah Carey, Michael Bublé") are separate artists, each with a page of its own and a link of its own; band names such as "Earth, Wind & Fire" stay whole.
- **Rename** genres, composers and years, and **merge** genres (several at once, too); only the songs' tags change.
- **Choose the columns** of song lists by right-clicking their headings (All Songs, Folders and album pages each remember their own); album pages show the full columns. Hovering a Quality cell shows the bit depth and sample rate.
- **Your Stats** starts with a **calendar**: click a day to see what you played and when.
- **Properties…** (formerly Edit Info) can be resized.
- The **Cover** window shows a cover large, with its size and where it comes from, and can **change**, **save** or **remove** it for an album or a song (also **Cover…** in every song's menu). FLAC, MP3, Ogg and Opus are changed directly in their tags, M4A and AIFF by FFmpeg without re-encoding, checked sample for sample.
- **Covers are kept** on disk and made ahead, so scrolling no longer waits for them, and scrolling big libraries is much smoother.
- The equalizer's **Delete** works again for your own presets.
- Everything new is in Japanese too when Settings → Appearance → Language is set to 日本語, and Song information from MusicBrainz (new in 2.1.0) is in every song's menu and on album pages next to **Properties…**.

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

Song information comes from [MusicBrainz](https://musicbrainz.org) and the people who contribute to it (core data CC0, genres CC BY-NC-SA 3.0); the app's MusicBrainz code is its own, written against MusicBrainz's published web service, and contains no code from MusicBrainz or Picard.

On Windows and Linux the apps use the [Inter](https://rsms.me/inter/) typeface by The Inter Project Authors (SIL Open Font License 1.1; its licence is in every package). The Python app plays sound through [miniaudio](https://github.com/irmen/pyminiaudio) (Irmen de Jong and David Reid, MIT License), the Rust app through [cpal](https://github.com/RustAudio/cpal) (Apache License 2.0).

The bundled FFmpeg executables are licensed separately (LGPL on Windows and Linux, GPL on macOS). See `FFMPEG-NOTICE.txt` inside the package.
