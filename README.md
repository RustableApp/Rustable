<p align="left"><img src="assets/rustable-mark.svg" width="80" alt="Rustable icon"></p>

# Rustable

**Your query took 30ms. Why is your app still thinking?**

Rustable is a native database client. We wanted to click a table and see the table, run a query and get the results, and edit a value without feeling like we’d opened an accounting suite. So we’re making one.

It’s written in Rust. Of course it is. We also use mise, and mise is written in Rust. That’s how you know it’s good. Please don’t ask any follow-up questions about this methodology.

**Still in development.** You can build from source; signed, packaged releases aren’t ready yet. There are rough edges and unfinished features. This isn’t a claim of complete TablePlus parity.

## What’s in it

- Saved connections and separate database windows.
- SSH tunnels, schema browsing, and credentials stored through the system credential store when enabled.
- Resizable data grids, sorting, filtering, pagination, and a row inspector.
- SQL tabs, syntax highlighting, completion tooling, query history, and favorites.
- Staged table edits, undo, and a SQL review before saving.
- Result exports, light and dark themes, and keyboard shortcuts for the things you do repeatedly.

Rust + egui means a native, GPU-rendered UI that can redraw at your monitor’s refresh rate during interaction. You bought the nice monitor. Might as well use it for something besides the mouse cursor. Vsync is enabled; idle windows don’t deliberately redraw continuously.

You already have a browser. No need to ship another one to display some rows.

## Database support

PostgreSQL and SQLite work today. MySQL, MariaDB, Redis, and more engines are planned; they aren’t implemented yet. Rustable is intended to be a client for your databases, not a two-engine club.

### Why start with PostgreSQL and SQLite?

Because PostgreSQL is the only correct choice for a server-side SQL database, and SQLite is obviously, objectively awesome. These are completely unbiased engineering conclusions.

We can have opinions and still support your toy databases. More engines are coming.

### What about Windows and Linux?

We’re even building first-class native versions for operating systems inferior to macOS, like Windows and Linux. Unlike certain database explorers that cost money and are closed sauce. Very generous of us.

## Run it

Use [mise](https://mise.jdx.dev/) for the toolchain and Cargo for dependencies:

```sh
mise run dev
mise run check
mise run build

# macOS app bundle: target/Rustable.app
mise run bundle
```

Or run `mise exec rust@stable -- cargo run --release`. The Cargo lockfile is checked in.

The connection window is the starting point. Choose an existing SQLite database or configure PostgreSQL. The app won’t create a new SQLite file because you mistyped a path.

## About the speed thing

We’re spending time on connection reuse, unnecessary network round trips, bounded results, and keeping database work off the UI thread. The grid renders visible rows instead of rendering your entire regrettable `SELECT *`.

The goal is to feel immediate. We’re not going to turn that into a made-up “100× faster” claim. Measurements and implementation details live in [the performance notes](docs/PERFORMANCE.md).

Found something slow? Please include the database engine, what you did, and whether SSH was involved. Query timings, UI timings, and network latency are different things; useful reports help us find the right one.

## A few things to know

Results have limits. Export operates on loaded results, not automatically on your whole database. Query history and saved SQL live in local app data and may contain sensitive text. SQL editing and table editing also have different transaction behavior; see the driver and editing notes before relying on a particular workflow.

- [Driver capabilities](docs/DRIVER_CAPABILITIES.md)
- [PostgreSQL editing](docs/POSTGRES_EDITING.md)
- [Language servers](docs/LANGUAGE_SERVERS.md)
- [Automatic updates: current state](docs/AUTOMATIC_UPDATES.md)

Built for people who want to get their work done. Not clowns. 🤡 (Clowns may build from source.)

## Handcrafted, obviously

Every line lovingly written by skilled artisans. Our artisan is called Astra and works suspiciously quickly.

Fine. It’s vibe coded. The source is here. The tests still have to pass.

## Isolated native QA

Build with `mise exec rust@stable -- cargo build --release --features qa` and copy the binary only to `target/Rustable QA.app/Contents/MacOS/rustable`. QA has its own window title and persistence namespace.

Build normal releases separately with `mise exec rust@stable -- cargo build --release`. Both use the same Cargo output path, so don’t copy the QA binary into the normal app bundle. Replace bundle executables atomically and restart only the intended app.
