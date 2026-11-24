# Rustable v0.4.39

- Save PostgreSQL edits much faster over remote and SSH connections: first-use preparation and saving use three network exchanges, and repeated saves can use just two.
- Open repeated SQL previews immediately using a bounded preparation cache. Saves still revalidate the complete schema under lock before writing.
- Save many rows and multiple tables without adding a network wait for each edit. Original-row checks, affected-row checks, atomic rollback and cancellation before commit remain enforced.
- Preserve safe recovery when schemas change or connections fail. Uncertain writes are never automatically replayed.

In a disposable PostgreSQL benchmark with 30 ms simulated network latency, preparation and saving improved from 987 ms to 145 ms uncached and 93 ms cached. Cached preparation took 0.22 ms. Actual timings depend on your database, network, locks and triggers.
