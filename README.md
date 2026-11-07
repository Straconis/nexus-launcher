# Nexus Launcher

Nexus Launcher is a native desktop launcher for the Straconis tool suite. It reads a local catalog, looks up each tool's latest public GitHub release, installs a build under your user account, and starts it from a dark card grid titled **NEXUS LAUNCHER**.

The window is a Rust app built with [egui](https://github.com/emilk/egui) and [eframe](https://github.com/emilk/egui/tree/master/crates/eframe). It does not need a GitHub token. Refresh re-reads `catalog/apps.toml` and checks `releases/latest` again.

```bash
git clone https://github.com/Straconis/nexus-launcher.git
cd nexus-launcher
cargo run --release
```

Check the catalog without opening a window or calling GitHub:

```bash
cargo run -- --check
cargo test
```

## Downloads

Tagged releases publish binaries on [GitHub Releases](https://github.com/Straconis/nexus-launcher/releases). Launcher packages include:

- `nexus-launcher-setup-<version>-windows-x86_64.exe` — Windows per-user installer
- `nexus-launcher-windows-x86_64.zip` — Windows portable build
- `nexus-launcher-linux-x86_64.tar.gz` — Linux binary and seed catalog
- `SHA256SUMS.txt`

The Windows installer creates **Start → Straconis Software → Straconis Software Dev Nexus Launcher**, installs under `%LOCALAPPDATA%\Programs\Straconis Software\Dev Nexus Launcher`, and adds an uninstall entry. It does not request administrator rights. Reinstalling preserves an edited `catalog/apps.toml`; uninstalling preserves that catalog, sign-in configuration, and managed tool data. The installer compiler is pinned to Inno Setup 6.4.3 with an upstream SHA-256 check.

Portable archives can be extracted anywhere writable; run the included executable. `--check` parses the catalog without opening the GUI. The window opens maximized; its restored size is 1440×900. **Preferences** in the compact account toolbar controls the charcoal header/footer and maximized window. Choices are saved to your profile.

The launcher works from its local catalog without a broker. Central administration needs a separately running broker and a reachable `server_url` in the user's `central.toml`; the installer does not bundle server credentials or point to the developer's local VM. See [Central catalog](#central-catalog). macOS binaries are not included.

The source version is **0.5.1-alpha**; consult the Releases page for published artifacts. Launcher self-update is integrated in this branch. Individual managed-app updates are available through **REFRESH** and **UPDATE**.

## Website tabs

**Latest Post** shows the newest public WordPress post. **Request a Project** switches between a project request and a general contact form, both protected by Cloudflare Turnstile. **Language Library** runs in the launcher and is available offline. **Straconis.dev** opens the full website in your default browser. The forms and latest post require Project Nexus 1.19.0 on the website. The footer mirrors the site’s confirmed Ko-fi tips.

Embedded pages use Microsoft Edge WebView2 on Windows and WebKitGTK 4.1 on Linux. On Debian and Ubuntu, install the Linux runtime with `sudo apt install libwebkit2gtk-4.1-0`. Embedded pages on Linux require X11 or XWayland.

On Windows, Stat Tracker’s card detects the installed copy after the setup program has finished. It reads the actual installation path registered by the installer, including custom folders.

## Central catalog

An optional broker, `nexus-server`, holds an explicit app list and per-user grants. GitHub login identifies people by numeric user id. Private builds are downloaded through the broker, so a regular user does not need access to the private source repository. Cards are not filtered only in the window: catalog, release, and download requests are checked on the server. The local `catalog/apps.toml` file still works when no broker is configured. Website repositories and WordPress plugins are not imported.

Setup, the device-flow choice, permissions, and what was not tested against a live GitHub app are in [docs/central-admin.md](docs/central-admin.md). Example environment names, with empty secrets, are in [deploy/nexus-server.env.example](deploy/nexus-server.env.example).

**Published `v0.1.0-alpha` and `v0.3.0-alpha` builds cannot update themselves.** Those downloads were built before self-update was integrated. Install the first release containing this change by hand (see [Updating Nexus Launcher](#updating-nexus-launcher)). After that, the window can install newer releases.

## Updating Nexus Launcher

The **LAUNCHER** strip in the window shows this build's version and whether a newer release of Nexus Launcher itself is available. **UPDATE LAUNCHER** downloads it. The check runs on a background thread. Nothing is downloaded until you press the button.

The check uses the GitHub release list for `Straconis/nexus-launcher`, not `releases/latest`. GitHub's latest endpoint hides prereleases, so an alpha build would never see the next alpha. A prerelease version (this tree is `0.4.0-alpha`) is offered a newer prerelease or a newer stable release. A stable version is not offered prereleases. The comparison is semver, after a leading `v` is removed. The same version is not offered again, and an older tag is not a downgrade. A tag that was already applied is remembered under the data directory so a replaced build cannot immediately install that same tag again.

Only the official archive for this machine is eligible:

| OS | Archive |
| --- | --- |
| Linux x86_64 | `nexus-launcher-linux-x86_64.tar.gz` |
| Windows x86_64 | `nexus-launcher-windows-x86_64.zip` |

Anything else (AppImage, deb, MSI, a macOS archive, another CPU) is ignored. The release must publish a SHA-256 digest or `SHA256SUMS.txt` that lists that archive. There is no "install anyway" for the launcher. A bad or missing checksum deletes the download and leaves the program you are running in place.

The swap replaces that one executable. It does not copy `catalog/apps.toml` over yours, and it does not touch installed tools under the data directory. On Windows the running file is renamed aside (a running `.exe` often cannot be overwritten in place) and the new file takes its name. If the folder is not writable, the update stops and does not ask for administrator rights. If the new file cannot be moved into place, the previous executable is restored. The new build is started with the same arguments and the old process exits. The next launch deletes the `.old` backup when the operating system allows it.

Limits:

- macOS has no published binary, so the strip says self-update is not available there.
- The tag in `Cargo.toml` is the version the binary reports. A release tag must be `v` plus that version or the running copy will not recognize itself and may offer the release you just installed. `v0.3.0-alpha` is already published without the updater; this integration uses the new version and tag `v0.4.0-alpha`.
- Published `v0.1.0-alpha` and `v0.3.0-alpha` builds do not contain this code. Users of those files need one manual download. Self-update starts with the first release built from this change.

## Optional tile banners

In the desktop admin editor, open a tool or supported project and paste a direct HTTPS PNG or JPEG URL into **Tile banner**, then save. For WordPress, upload the image to **Media → Add New** and copy its **File URL**. The same setting is available in WordPress **Settings → Nexus Service**.

Use a wide 3:1 image up to 4 MiB and 4096 pixels per side. Images load in the background and crop to fit the card. Refresh retries failed downloads. A blank URL restores the default appearance. S3D Labs uses its hosted banner with a bundled fallback.

Custom banners require Nexus Service 0.2.0 or the 0.4.0-alpha Rust broker. Updating the desktop and the service enables banner editing and persistence.

## What it is

Each catalog entry is one desktop tool. The launcher asks the unauthenticated GitHub API for that repository's latest release, picks a build for the machine you are on, and shows one card:

- **COMING SOON** when the repo has no published release (HTTP 404). A missing repository looks the same, because the request is unauthenticated.
- **INSTALL** when a release asset matches this operating system and CPU.
- **LAUNCH** when a portable build or archive is already installed and the launch file is still on disk. An installed build remains launchable if the latest release has no compatible asset.
- **UPDATE** when the installed tag is older than the latest release.
- **OPEN INSTALLER** when the file is an MSI, DMG, deb, or setup executable that was downloaded and handed to the operating system.

The seeded catalog lists Stat Tracker, SnipScratch, SnapLapse, and FrameWeaver. Those cards stay on **COMING SOON** until the matching GitHub repositories publish a release. Add another `[[app]]` table and press **REFRESH**. You do not recompile the launcher to add a tool.

## Architecture inspiration

The shape of the app — a local catalog of desktop tools, installs driven by published releases, and a card grid that launches them — takes a cue from ArtCraft. That is architecture inspiration only. This repository does not include ArtCraft trademarks, logos, copy, or assets, and it is not affiliated with ArtCraft.

## Design

The header pairs a dedicated Straconis Nexus Launcher graphic on the left with the Straconis Dev / Project Nexus logo on the right. Footer brand links open `https://straconis.dev` from the standard Software wordmark and `https://s3dlabs.com` from the S3D Labs logo supplied from `Discord Server Logo.png`. Hovering the Software wordmark also shows the full developer logo. Images in `assets/branding/` are embedded in the binary so portable builds need no external image files. The current palette remains provisional. Header and button labels use [Orbitron](https://github.com/theleagueof/orbitron) (SIL Open Font License, reserved font name "Orbitron"). The file in `assets/fonts/` is the unmodified variable font, with `assets/fonts/OFL.txt`. Body copy stays on egui's default UI font so descriptions stay readable. Calls to action are uppercase.

| Token | Hex | Use |
| --- | --- | --- |
| accent | `#ff244e` | Primary buttons, rules, category labels |
| bg | `#07080b` | Window background |
| panel | `#101118` | Cards and dialogs |
| text | `#f5f5fa` | Primary text |
| muted | `#b4b4c5` | Summaries and secondary lines |
| border | `#64203c` | Card and panel strokes |

Cards show up to three programming languages reported by GitHub, ordered by source-code size. Public repository language lookups use the same unauthenticated API budget as release checks; private repository languages are fetched through the access-checked broker. These labels describe source code, not runtime requirements.

## Catalog

`catalog/apps.toml` is the extension point. `[[app]]` tables are the entries. Unknown keys are ignored. `nexus-launcher --check` parses the file, prints each id, and exits. It does not contact GitHub.

| Field | Required | Meaning |
| --- | --- | --- |
| `id` | yes | Lowercase slug, unique in the file. Example: `stat-tracker`. |
| `name` | yes | Name shown on the card (rendered in uppercase). |
| `summary` | no | One or two sentences under the name. |
| `repo` | no | GitHub `owner/name`. A trailing `.git` is stripped. Omit it, or leave it blank, and the card stays **COMING SOON** with no network call. |
| `homepage` | no | URL opened by **REPOSITORY**. If omitted, the release page is used once a release exists; otherwise the configured GitHub repository is used. |
| `category` | no | Short label above the name, shown in the accent color. |
| `executable` | no | Relative path of the program inside an extracted archive. No absolute paths and no `..`. |
| `asset_preference` | no | Case-insensitive substrings. The first one that matches a compatible asset wins. |
| `platforms` | no | `linux`, `windows`, and/or `macos`. Empty means every platform. |

```toml
[[app]]
id = "stat-tracker"
name = "Stat Tracker"
summary = "Session and project statistics for the Straconis desktop suite."
repo = "Straconis/stat_tracker"
category = "insights"
homepage = "https://github.com/Straconis/stat_tracker"
# asset_preference = [".AppImage", ".tar.gz"]
# executable = "stat-tracker"
# platforms = ["linux", "windows", "macos"]
```

Asset choice, after checksum files are ignored:

1. Drop anything that is not for this OS, and drop a file whose name names a different CPU (`x86_64` / `aarch64`).
2. Prefer a portable file (including AppImage), then a zip or tar.gz, then an installer.
3. On Linux, an AppImage outranks other portable files.
4. `asset_preference` overrides that order when a substring hits.

Downloads are limited to HTTPS on `github.com`, `api.github.com`, `codeload.github.com`, and `githubusercontent.com` hosts, including redirects.

## Build

Rust **1.99.1** is pinned in `rust-toolchain.toml`. [rustup](https://rustup.rs/) installs it on the first `cargo` command.

Linux packages used to compile and link eframe (glow / winit):

```bash
sudo apt-get install -y pkg-config libssl-dev libgtk-3-dev libwebkit2gtk-4.1-dev \
  libx11-dev libxi-dev libxcursor-dev libxrandr-dev libxinerama-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libgl1-mesa-dev
```

Then:

```bash
cargo run --release
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

| Flag | Effect |
| --- | --- |
| `--check`, `-c` | Parse the catalog, print it, exit. No window and no GitHub traffic. |
| `--catalog PATH` | Use this `apps.toml` instead of the default. |
| `--help`, `-h` | Print usage. |

The default catalog is `catalog/apps.toml` in the working directory. If that file is missing, the launcher also looks next to the executable and in `target/release`'s parent tree, so `cargo run --release` from a checkout finds the seed file.

## Platforms

The same code paths cover Linux, Windows, and macOS. Install roots come from the [`directories`](https://crates.io/crates/directories) crate (`org` / `Straconis` / `nexus-launcher`):

| OS | Data root | Apps |
| --- | --- | --- |
| Linux | `$XDG_DATA_HOME/nexus-launcher` or `~/.local/share/nexus-launcher` | `<root>/apps/<id>/` |
| macOS | `~/Library/Application Support/org.Straconis.nexus-launcher` | `<root>/apps/<id>/` |
| Windows | `%APPDATA%\Straconis\nexus-launcher\data` | `<root>/apps/<id>/` |

Each app directory holds `install.json` and a `current/` folder. Launch records the real file path. This environment runs the test suite on Linux. Windows and macOS paths are implemented and covered by pure tests (archive safety, asset selection, catalog parsing), not by a GUI run on those systems.

## Installs, checksums, and limits

**Portable files and AppImage.** The file is stored under `current/`, marked executable on Unix, and **LAUNCH** spawns it. An AppImage still needs a working AppImage runtime (FUSE or an extracted runtime) on the host. The launcher does not bundle one.

**zip and tar.gz.** Extracted under `current/payload/`. The launch target is `executable` from the catalog, otherwise the only file in the archive, otherwise the best name or executable-bit match for the app id. Symlinks and hard links are rejected. Paths that escape the destination (`..`, absolute paths) abort the extract and delete the partial tree. `tar.bz2`, `tar.xz`, and `tar.zst` are recognized so they are not silently skipped, then refused with an unsupported-archive error. v1 unpacks zip and gzip tarballs only.

**MSI, DMG, deb, and setup executables** (the file name contains `setup` or `install`). These are downloaded into `current/` and opened with the operating system (`xdg-open` on Linux, `open` on macOS, the shell association on Windows via the `open` crate). Nexus Launcher does not run them silently, does not elevate, and does not know whether the system installer finished. The card stays **OPEN INSTALLER**. **LAUNCH** is only for builds whose files the launcher itself unpacked.

**SHA-256.** Verification runs when the GitHub asset `digest` is `sha256:<hex>`, or when the release includes `SHA256SUMS` / `SHA256SUMS.txt` / `checksums.txt`, or a sidecar named `<asset>.sha256`. A mismatch deletes the download. If none of those are published, **INSTALL** opens a dialog and waits for **INSTALL ANYWAY**. There is no signature, notarization, or SmartScreen handling. The operating system can still warn.

**Updates.** **REFRESH** compares the recorded tag with `releases/latest`. A newer semver tag (a leading `v` is ignored) shows **UPDATE**. Non-semver tags count as an update when the strings differ. The previous `current/` directory is swapped out after the new files are in place. Replacing a portable build can fail if that process is still running and the OS locks the file, which is common on Windows.

**GitHub rate limit.** Requests send `User-Agent: nexus-launcher/<version>` and no token. The unauthenticated budget is 60 requests per hour per IP. A banner appears when a response is rejected for rate limit or when fewer than 10 requests remain. A hard limit stops the rest of that refresh instead of spending more calls. Private repositories and repos with no release both surface as **COMING SOON**. The central broker is separate and uses its own server credential for private release reads.

**Best-effort install state.** `install.json` plus the launch file means installed. A missing launch file is an incomplete install, and the card offers **INSTALL** again. An installer that was only handed to the OS is "installer downloaded", not "installed".

## Why there is no system tray in v1

A tray icon needs `tray-icon` (or the platform shell) beside eframe's event loop. That combination is uneven across Windows, macOS, and Wayland, and it is easy to get a window that will not reopen. v1 is a window you open when you want a tool. Launch-at-login and a tray can land later without changing the catalog format.

## Tests

`cargo test` does not open a window and does not call GitHub. It covers catalog validation, the `--check` flag, release JSON and rate-limit classification, asset selection, SHA-256 sums, zip and tar.gz extraction, zip-slip rejection, and install-record detection. Self-update tests cover prerelease selection (including an alpha that `releases/latest` would hide), stable builds ignoring newer alphas, missing checksums, the official Linux and Windows archive names, a failed checksum, a zip-slip archive, an archive with no launcher binary, rollback when the new file cannot be moved into place, and replacement of an open executable without touching a custom catalog or installed apps. Those replacement tests run on Linux and in Windows CI. The read-only directory case runs on Linux, where a mode of `0555` can refuse the write without granting extra privileges.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

The Orbitron font is bundled under the SIL Open Font License. See [assets/fonts/OFL.txt](assets/fonts/OFL.txt). That license applies to the font files only.

## Roadmap

- An optional GitHub token in the OS keyring, for a higher rate limit. v1 never reads or stores a token.
- System tray and launch at login, once the install and launch loop has settled.
- Release notes inside the card.
- Detecting whether a deb or MSI actually finished installing.
- Signature checks (minisign or sigstore) in addition to SHA-256.
- `tar.zst` and `tar.xz` extraction.
- Per-app launch arguments in the catalog.
