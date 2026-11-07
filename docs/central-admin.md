# Central catalog and per-user access

This document describes the `0.3.2-alpha` broker. Release packaging includes a Windows per-user installer; see GitHub Releases for published artifacts. Launcher self-update is integrated alongside central catalog administration.

## What it is

Steve can run one Nexus Launcher, sign in as an admin, and open **MANAGE** for a resizable, scrollable app-management window:

- add desktop apps to a shared catalog, one at a time
- edit existing entries without changing their published state
- show or hide them using **SHOW IN LAUNCHER** / **HIDE FROM LAUNCHER**
- grant or revoke a GitHub user on a single app

Other people sign in with GitHub. The broker returns only the apps they may see. A private build is downloaded through the broker. The person does not need access to the private source repository.

The local file `catalog/apps.toml` still works with no broker configured. When a broker is configured, its catalog is authoritative: local seed rows do not override central edits, and unpublishing removes the card on the next catalog refresh. Anonymous users receive the published public catalog. Nothing scans a GitHub org or imports website repositories or WordPress plugins. If an app is not an explicit catalog row, it is not a desktop app.

## Launcher help and listing controls

**MANAGE** opens the listings first. Each row says **SHOWN IN LAUNCHER** or **HIDDEN FROM LAUNCHER**. Show/hide controls affect the shared catalog, independently of public or restricted access. New apps start hidden; save their details, then choose **SHOW IN LAUNCHER**. **ADD APP** and **EDIT** open the details form; **USER ACCESS** opens grants. **HELP / HOW TO USE** opens searchable help from the main window, and **HELP WITH LISTINGS** opens it from administration.

## Categories, GitHub import, and summaries

App details use a category dropdown with parent/subcategory paths, such as **Hardware › Electronics**. Choose a parent in **MANAGE CATEGORIES** to add or move subcategories. Renaming a parent updates the branch and its app assignments; removing a parent removes its subcategories and clears their assignments. **MANAGE CATEGORIES** stores shared categories in the broker database. Add, rename, or remove a category there; rename updates assigned entries and removal clears their category without hiding the apps.

Paste a GitHub link in App details to populate the canonical `owner/repo`, homepage, slug ID, and display name. New `stat_tracker` repositories become `stat-tracker` IDs and “Stat Tracker” display names. Existing IDs and custom display names are preserved when editing. The repository URL can include `.git`, query parameters, fragments, or a subpage path.

**DRAFT SUMMARY FROM GITHUB** fetches the description and README introduction through an admin-only broker endpoint. Review the suggestion or choose the README introduction, edit the multiline summary, then save separately. These are source-assisted drafts, not AI rewrites. A server release token is required for private repository sources. The token stays on the broker.

## Status stamps and supported projects

The **Project stamp** dropdown on each admin row saves immediately. **Project status** in App details also applies a colored stamp when the details are saved. **MANAGE STATUS STAMPS** adds labels, renames/recolors selected stamps, or removes them. Defaults are Released (green), In development (amber), and Seeking funding for prototype (cyan). Stamp configuration is shared in SQLite. Renaming/recoloring updates assigned cards after refresh; removing a stamp clears assignments. Stamps do not alter listing, access, or download availability.

**SUPPORTED PROJECTS** is a separate launcher page. Supported entries have a separate **Project type** dropdown with defaults Twitch stream, Podcast, and Community. **MANAGE PROJECT TYPES** adds, renames, or removes types independently of software/hardware categories. Type renames update supported entries only; removing a type clears their type without hiding the project. Admins use **MANAGE SUPPORTED PROJECTS** to add, edit, show/hide, or remove external project cards. These use the existing authorized catalog backend with `kind = "supported"`, a required HTTPS homepage, and public visibility. They show **VISIT PROJECT**, not software install/update actions. Supported-project IDs share the app ID namespace; changing a listing's kind through an existing ID is rejected.

Cards also show up to three GitHub language badges, sorted by source-code byte counts. Public language lookups use the unauthenticated GitHub budget; private lookups require access through the broker. Language badges are independent of project-status stamps.

## App updates

**REFRESH** looks up the latest release; installed apps get **UPDATE** when a newer compatible build is available. Portable apps are downloaded, checksum-verified, staged, and swapped into the managed install. Installer packages are handed to the OS for user interaction. This does not silently update arbitrary system installations. The separate launcher self-updater in PR #2 remains outside this branch.

## Module map

| Piece | Role |
| --- | --- |
| `crates/nexus-protocol` | Serde types for the HTTP API. No I/O and no secrets. |
| `crates/nexus-server` | Self-hosted process. SQLite, GitHub device flow, sessions, catalog, grants, release proxy. |
| `crates/nexus-server` `policy` | Slug, `owner/name`, login, and homepage checks before storage or fetch. |
| `crates/nexus-server` `auth` | Device-flow handshake and session hashes. |
| `crates/nexus-server` `github` | Server-only GitHub calls and host allow list. |
| `crates/nexus-server` `routes` | Catalog, release, download, and admin handlers. Each one checks access. |
| `src/central` | Desktop client, server URL file, session storage. |
| `src/central_panel.rs` | Account and admin widgets. |
| `src/ui.rs` | Starts broker work on the existing background thread and draws central cards. |
| `src/install.rs` | `install_verified_file` installs bytes the broker already checked. It does not accept an arbitrary URL. |

The desktop does not embed the OAuth client secret, the device code, or the release token.

## Auth

GitHub's OAuth **device flow** is mediated by the broker. It was chosen because a public callback URL is not selected yet. The authorization-code flow plus a loopback redirect needs a registered callback on a stable hostname and a local HTTP listener. Device flow does not. The desktop must not hold the client secret, so the desktop does not talk to GitHub's token endpoint.

1. The launcher asks `POST /v1/auth/device/start`.
2. The broker calls GitHub with the OAuth client id and asks only for the `read:user` scope.
3. The device code stays in SQLite. The launcher receives a random poll id, the user code, and the verification URL.
4. The launcher opens the browser and polls `POST /v1/auth/device/poll`.
5. The broker sends the client id, client secret, and device code to GitHub.
6. On success it reads `GET /user`, keeps the numeric GitHub id, and drops the GitHub access token.
7. It returns a broker session token once. Polling the same id again returns `expired`.
8. Sign-out deletes the server session and the local copy.

Admin is not a flag the client can set. `NEXUS_ADMIN_GITHUB_IDS` is a list of numeric GitHub user ids. The broker checks that list on every authenticated request. Logins are not used for admin because they can change.

Sessions last `NEXUS_SESSION_TTL_HOURS` (default 30 days, maximum 90). There is no refresh token. Sign in again when the session ends.

The launcher stores the session in the OS keyring (`keyring` crate: service `nexus-launcher`, account `central-session`). Native credential backends are explicitly enabled for Linux, Windows, and macOS. Linux uses the kernel keyring, whose lifetime is tied to the OS session; sign in again if that keyring is cleared. If the keyring cannot save, it falls back to `session.token` in the config directory, mode `0600` on Unix. That file is a weaker store. This environment did not prove a live keyring prompt.

## Access rules

| Caller | Catalog | Release metadata | Download |
| --- | --- | --- | --- |
| Anonymous | Published public apps | Public apps only | Public apps only |
| Signed-in user | Those, plus published private apps they are granted | Same | Same. Another user's grant returns 403 |
| Admin id from the environment | Published public and private apps | Published apps | Published apps, including private ones without a personal grant |
| Anyone | Unpublished apps are hidden | Unpublished is 404 | Unpublished is 404 |

A direct request is checked again. A catalog response is not a download permit.

Private catalog rows omit `repo`. Admin list responses include it so the owner can edit the row.

Public apps may still be installed by the launcher from GitHub, the same way as a local catalog entry. Private apps are only fetched through `GET /v1/apps/{id}/download`. The response is the file bytes plus `x-nexus-sha256`. The launcher hashes the body and refuses a mismatch. The broker also hashes before it sends. The GitHub URL and the release token are not in the response.

## Offline copies

Revocation removes the app from later catalog responses and makes a new download return 403. It does not delete a build already installed under the user data directory. The broker has no remote-wipe path. Say that to anyone who receives a private build.

The launcher does not keep a private catalog file that could be reused as a download credential. Launching an app that is already installed does not need the broker.

## What the owner configures

These steps are the intended setup. They were **not** executed against a live GitHub OAuth app or a deployed host in this change.

1. Create a GitHub OAuth App (Settings → Developer settings → OAuth Apps). Enable Device Flow. The callback URL can be a placeholder such as `http://127.0.0.1` because device flow does not use it. Copy the client id and generate a client secret.
2. Create a fine-grained personal access token, or a GitHub App installation token, that can read release assets for the private app repositories only. Do not grant administration. Public apps do not need this token. Private downloads fail closed without it.
3. Copy `deploy/nexus-server.env.example` to a file outside the git tree. Fill in the values. Do not commit them.
4. Put the numeric GitHub user id in `NEXUS_ADMIN_GITHUB_IDS`. Resolve it with `https://api.github.com/users/<login>` and read `id`, not the login string.
5. Build and run the broker on a machine you control:

   ```bash
   cargo run -p nexus-server --release
   ```

   The default listen address is `127.0.0.1:8787`. Put TLS in front before exposing it. Device codes and session hashes sit in the SQLite file; keep that file private (`0600` on Unix).
6. On each launcher, create the config file printed at startup's config directory (`central.toml`):

   ```toml
   server_url = "https://your-broker.example"
   ```

   No token belongs in that file. `http://` is accepted for a local broker. A URL with a username or password is rejected.

## Security limits

- Live GitHub login, a real OAuth app, and a real private release download were not exercised here. Tests use an in-process fake GitHub.
- Device flow can be phished if someone is persuaded to enter the code on an attacker's screen. That is a property of device flow.
- The broker should be reached over HTTPS except on loopback. Sessions are bearer tokens.
- Reqwest is relied on not to forward `Authorization` to a different host after a redirect. This tree allows only GitHub HTTPS hosts and attaches the release token only when the URL host is `api.github.com`. That redirect behavior was not re-tested against live GitHub.
- The SQLite connection is a process-wide mutex. That is enough for a small single-host broker, not a multi-server deployment.
- Admin can download a published private app without a personal grant, so they can try the install. They cannot do that for an unpublished app.
- A stolen admin session can add a `owner/name` repo. Fetches still go only to the GitHub API for that slug, not to an arbitrary URL.
- Checksums are required. A release asset needs a `sha256:` digest or a `SHA256SUMS.txt` asset. There is no "install anyway" on the broker.
- Already downloaded files cannot be revoked.
- OS keyring storage was not verified on a desktop session. Windows Credential Manager and a Linux secret service were not exercised by hand.
- The window was not clicked through in a desktop session in this environment.

## Tests that do run

`cargo test -p nexus-server` covers anonymous, member, and admin callers; grant and revoke; a second user's direct download and release request; unpublished apps; slug and repo rejection; an asset URL that is not a GitHub host; a checksum mismatch; secret strings absent from error bodies; and a device poll that cannot be replayed.

Desktop tests cover the server URL rules, the session file mode on Unix, and redaction of the session token in `Debug`.

### Public release metadata

Public app releases and language badges are cached in the broker for five minutes. The broker uses its OAuth application credentials for public GitHub metadata when no release token is configured; credentials stay on the server. Previously cached metadata remains usable if GitHub is temporarily unavailable. Visibility is checked before serving the cache. Desktop installations without a central server still use GitHub's anonymous API limits.

### Public host deployment

Use a Linux host with a DNS hostname pointing to it and incoming ports 80/443 available. Keep broker port 8787 on loopback behind Caddy; `deploy/Caddyfile` provides HTTPS termination. Set `NEXUS_DOMAIN` in Caddy's service environment to your hostname.

Install the compiled `nexus-server` at `/opt/nexus/nexus-server`, create a dedicated `nexus` system user, and create `/var/lib/nexus` owned by that user. Store the environment file at `/etc/nexus/server.env` readable only by root and the nexus group. Set `NEXUS_BIND=127.0.0.1:8787` and `NEXUS_DATABASE=/var/lib/nexus/nexus.sqlite`. Install `deploy/nexus-server.service` into systemd.

To preserve the current catalog, stop the existing broker and make a SQLite backup before transferring the database to `/var/lib/nexus/nexus.sqlite`. Set owner nexus:nexus and mode 0600. Transfer existing OAuth application configuration privately; no credentials belong in the desktop installer or repository. Start the service and verify `/v1/health`, `/v1/catalog`, and a public app's `/v1/apps/{id}/public-metadata` through HTTPS before configuring launchers.

In launcher Preferences, save the HTTPS service address and restart. The existing 0.3.0-alpha release can use the same address by placing `server_url = "https://YOUR_HOST"` in `central.toml` in its per-user configuration directory. Switching service addresses clears the saved session; sign in to the new service when admin access is needed. Public browsing does not require sign-in.
