# Rustable v0.4.1

Saved server connections now remember passwords automatically in the system credential store when you Save or Connect. The Remember password checkbox has been removed from PostgreSQL, MySQL, MariaDB, and Redis connection forms.

Existing connections adopt this behavior the next time they are saved or connected. Passwords remain outside preferences; secure-store failures still report session-only storage. The website demo is updated from this same release tag.
