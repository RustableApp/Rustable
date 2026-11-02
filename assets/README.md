# Rustable artwork

- `rustable-mark.svg`: original vector logo.
- `rustable-github-avatar.png`: matching app-style tile on GitHub dark gray, for the organization profile picture.
- `rustable-github-1024.png` / `rustable-github-2048.png`: alternative rusty R on a fully transparent background.
- `rustable-github.svg`: editable transparent logo.
- `rustable-logo-1024.png`: app-style tile with transparent rounded corners.
- `rustable-logo-2048.png`: larger PNG export.
- `rustable-macos-1024.png`: macOS runtime icon with a centered 824px tile and transparent margins.
- `Rustable.icns`: matching macOS bundle icon, rendered at each resolution from the vector mark. Regenerate both macOS assets with `python3 scripts/generate-macos-icon.py` (requires `resvg` and Apple's `iconutil`).
- `social-preview.png`: 1280 × 640 social card for GitHub and website links.
- `social-preview.svg`: editable vector source for the social card.

The PNG files are rendered directly from vectors with resvg, not enlarged from a small bitmap. The website supplies Open Graph and Twitter card metadata. GitHub’s repository social preview can use the same PNG in repository Settings → General → Social preview.
