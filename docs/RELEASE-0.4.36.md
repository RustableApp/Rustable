# Rustable v0.4.36

- The SQL editor now replaces Run with a prominent red **Cancel query** button while work is running. Once cancellation is requested, it shows **Cancelling…** until the operation finishes.
- PostgreSQL keeps its query monitor, where you can cancel the current workspace operation or a selected server query. Other databases request cancellation directly. Cancelling a batch stops remaining statements; already committed changes stay committed.
- Cancellation remains available in the connection toolbar when the editor controls are hidden, without duplicate controls when the editor is visible.
