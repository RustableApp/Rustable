# Rustable v0.4.3

Saved server connections now remember passwords automatically in the system credential store when you Save or Connect. The Remember password checkbox has been removed from PostgreSQL, MySQL, MariaDB, and Redis connection forms.

Existing connections adopt this behavior the next time they are saved or connected. Passwords remain outside preferences; secure-store failures still report session-only storage. The website demo is updated from this same release tag.

macOS application menu labels now consistently use Rustable in About, Hide, and Quit, including when launched directly from the executable.

The desktop connection manager no longer reserves an empty toolbar above the connection list. Browser navigation and visible notices remain available.

The connection Save button is disabled until settings or the password change, and disables again after saving.

v0.4.1 and v0.4.2 were unpublished build candidates, superseded by this release.

## TablePro import

Import… → From TablePro… now reads plaintext and encrypted `.tablepro` connection exports. Includes format versions 1 and 2, nested groups, duplicate handling and optional secure-store credential import. Supported engines are PostgreSQL, MySQL, MariaDB, SQLite and standalone Redis. Unsupported advanced settings are flagged for manual setup. See [compatibility details](TABLEPRO_IMPORT.md).
