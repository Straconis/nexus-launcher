# Central catalog and per-user access

This document describes the `0.2.0-alpha` broker. It is not a released build. Tagging and publishing stay unapproved. The self-update work on pull request #2 stays on `0.1.1-alpha` and is not part of this branch.

## What it is

Steve can run one Nexus Launcher, sign in as an admin, and open **MANAGE** for a resizable, scrollable app-management window:

- add desktop apps to a shared catalog, one at a time
- edit existing entries without changing their published state
- publish or unpublish them
- grant or revoke a GitHub user on a single app

Other people sign in with GitHub. The broker returns only the apps they may see. A private build is downloaded through the broker. The person does not need access to the private source repository.

The local file `catalog/apps.toml` still works with no broker configured. When a broker is configured, its catalog is authoritative: local seed rows do not override central edits, and unpublishing removes the card on the next catalog refresh. Anonymous users receive the published public catalog. Nothing scans a GitHub org or imports website repositories or WordPress plugins. If an app is not an explicit catalog row, it is not a desktop app.

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
