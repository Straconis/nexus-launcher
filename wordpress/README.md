# Straconis Nexus WordPress service

The PHP plugin replaces the separately hosted Rust broker on a WordPress host. The desktop API stays compatible with Nexus Launcher 0.3.0-alpha, including root `/v1` endpoints. Install it beside Project Nexus; it does not replace the homepage plugin or theme.

The installable plugin lives in `nexus-service/`. The optional `hostinger-bridge/` entry point supports a Hostinger subdomain rooted in its own folder below the existing WordPress root. See the plugin's readme for configuration and cache exclusions.

## Verification

The integration suite uses a disposable WordPress database with explicit opt-in. **It clears the plugin records and resets plugin list options on that test installation. Do not run it against a real catalog.**

```bash
NEXUS_TEST_ALLOW_WRITE=yes NEXUS_TEST_WP_ROOT=/path/to/disposable/wordpress php wordpress/tests/install.php
NEXUS_TEST_ALLOW_WRITE=yes NEXUS_TEST_WP_LOAD=/path/to/disposable/wordpress/wp-load.php php wordpress/tests/integration.php
```

`install.php` creates a config only when none exists. Test database overrides are `NEXUS_TEST_DB_NAME`, `NEXUS_TEST_DB_USER`, `NEXUS_TEST_DB_PASSWORD`, and `NEXUS_TEST_DB_HOST`. Copy the plugin directory into that disposable site's `wp-content/plugins` first. CI uses WordPress 6.8.3 / PHP 8.1 / MariaDB 11. Local verification also passed on WordPress 6.8.3 / PHP 8.4 and the user's WordPress 7.1.3 / PHP 8.3 container using a separate disposable table prefix.

The suite makes 60 assertions covering permissions, grant revocation, Unicode summary limits, catalog import validation, cached/stale metadata, editable categories/types/stamps, device flow, session expiry/logout, checksums, binary responses, and credential stripping / host validation on redirects. External GitHub requests are mocked in the repeatable suite. Separately, live authenticated metadata and the 10-entry migrated catalog were verified through HTTP on the actual local site, including the Hostinger-style bridge.

No credentials, databases, migration data, or site backups belong in the public plugin archive.
