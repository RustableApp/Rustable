# Rustable v0.4.38

- Save multiple row deletions with grouped SQL statements, reducing database round trips while retaining concurrency checks and atomic saves.
- Choose Silent, Alert or Safe query protection from the redesigned connection lock menu. Protection applies when sending changes, so you can keep editing locally.
- Confirm execution in one dialog, with **⌘Enter** to send. Silent mode saves and runs immediately; the Preview button opens the same SQL dialog when you want to inspect changes.
- Unlock Safe Mode using your database password or **Touch ID** on supported Macs. Enable Touch ID in Settings.
- Open the redesigned Settings with **⌘,** or the macOS application menu to choose your theme and security preferences.

Production connections start in Alert Mode 2; other connections start in Silent Mode. Protection choices currently reset on reconnect.
