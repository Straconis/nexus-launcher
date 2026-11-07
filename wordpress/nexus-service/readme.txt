=== Straconis Nexus Service ===
Contributors: Straconis
Requires at least: 6.5
Requires PHP: 8.1
Stable tag: 0.2.0
License: GPL-2.0-or-later

Central catalog, GitHub device sign-in, and authenticated cached metadata for Straconis Nexus Launcher. Runs in PHP with the existing WordPress database; no background Rust service is required.

== Installation ==
1. Back up WordPress and its database. Upload nexus-service-0.2.0.zip through Plugins > Add New > Upload Plugin. Activate Straconis Nexus Service.
2. Open Settings > Nexus Service. Enter the existing GitHub OAuth client id and secret (Device Flow enabled), and your numeric GitHub admin id. Private releases additionally require a repository-scoped server credential. Blank password fields retain existing credentials. No credentials are bundled in the ZIP.
3. Paste nexus-catalog-import.json into Import existing catalog to migrate the existing entries, categories, project types, and stamps. Import data is private administrative data; do not put it in a web-accessible directory.
4. Configure nexus.straconis.dev in Hostinger and enable HTTPS. DNS already points to straconis.dev via CNAME. The Hostinger hostname must serve the same WordPress installation, not an empty default folder or a redirect to the main site.
5. If Hostinger assigns the subdomain its own folder (normally public_html/nexus), place the separate Hostinger bridge index.php and .htaccess in that folder. The bridge expects the parent public_html directory to contain the existing WordPress installation. It serves only /v1/* API calls; it does not copy WordPress or alter the primary site's URL. Adjust the fixed parent include only if your folder layout differs.
6. Confirm https://nexus.straconis.dev/v1/health returns JSON with status ok. Then verify /v1/catalog and /v1/apps/stat-tracker/public-metadata. The WordPress-native equivalent is /wp-json/nexus/v1/health on the primary site.
7. Exclude /v1/* and /wp-json/nexus/v1/* from Hostinger/LiteSpeed and Cloudflare caching, redirect rules, and interactive browser challenges. Preserve Authorization headers. Use Cloudflare Full (strict) TLS after Hostinger has a valid origin certificate. Do not use Flexible TLS.
8. Configure the launcher service address as https://nexus.straconis.dev, then restart. Existing 0.3.0-alpha Windows builds read central.toml from %APPDATA%\Straconis\nexus-launcher\config. Newer source builds include the service-address field in Preferences.

== Server secrets ==
Credentials may instead be defined in wp-config.php as NEXUS_GITHUB_CLIENT_ID, NEXUS_GITHUB_CLIENT_SECRET, NEXUS_GITHUB_RELEASE_TOKEN, and NEXUS_ADMIN_GITHUB_IDS. Constants override the settings page. Secret inputs never display stored values. Settings saved through WordPress remain private database options with autoload disabled. Restrict access to database backups.

== API and access ==
/v1/health, /v1/catalog, /v1/auth/device/start, /v1/auth/device/poll, /v1/auth/logout, /v1/account, public metadata, language badges, release metadata, verified downloads, app administration, grants, categories, project types, status stamps, and source-assisted summary previews follow the launcher API.

Public browsing requires no sign-in. Private and hidden entries are checked before every response, including cache hits and downloads. Private repositories are omitted from client catalog responses. GitHub user tokens are discarded after identity verification; launcher sessions last 30 days, with only hashes persisted. Removing an admin id takes effect on the next request.

Public release metadata is cached for five minutes; cached data is retained as stale fallback during an upstream outage. A per-repository lock limits duplicate refresh requests. Outbound credential-bearing requests never follow redirects. Download redirects allow only specific HTTPS GitHub asset hosts and strip credentials; downloads require a SHA-256 match and are capped at 256 MiB. Your hosting execution-time and download limits can be lower.

WordPress administrators can edit listings directly in Settings > Nexus Service. Full category/type/stamp management and repo-assisted drafts are available in the launcher's management windows. App ids remain stable for installed application records.

== Removal ==
Deactivation stops the API. Catalog data and settings are retained to support reactivation; deleting the plugin does not automatically erase catalog data. Revoke OAuth credentials separately if retiring the service.
