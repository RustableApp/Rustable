# v0.3.2 release verification

## Local validation

- 475 native tests passed; 111 fixture/manual tests ignored.
- Strict native all-target Clippy passed; four focused Redis demo tests passed
  again after its iterator lint correction.
- Five packaging tests passed for the icon bundling change.
- Optimized native icon-fix build passed. A fresh local macOS bundle names
  Rustable.icns in Info.plist and includes the exact repository icon asset.
- Browser-package locked WASM check passed (existing browser warnings remain).
- Redis demo browser acceptance includes hash inspection/commands, a successful
  write, and retained data after switching Redis → SQLite → Redis.

## Publication gates

Tagged packages, signing/notarization, archive verification, live matching demo,
and stable download/update-feed verification are pending.

Native Windows/Linux GUI acceptance, Intel macOS and installed old-to-new Sparkle
update acceptance remain unverified. Redis demo is a bounded in-memory subset;
real Redis server support is unchanged from v0.3.1.
