# Isolated Redis command workflow assessment

Final Redis release assessment, 2026-11-08. Transactions and blocking commands
remain unsupported. This records the requested evaluation, not implemented parity.
Ordinary Workbench batches remain sequential and non-atomic.

## Existing paths and concrete gaps

`Session::execute_report` validates the whole batch before sending its first
command, then uses bounded responses on the workspace's single worker connection.
There is no per-command cancellation handle. Admitting MULTI/EXEC/WATCH here could
leave session state affecting later browser jobs; admitting a blocking command
would occupy the same worker. A finite server timeout does not provide explicit
user cancellation or resolve an interrupted write's outcome.

`redis_stream::Control` does own a dedicated socket and can shut it down, including
through the verified TLS/SSH transport. That is a useful foundation, but its
`start` API deliberately permits only MONITOR/SUBSCRIBE/PSUBSCRIBE/PUBSUB. Its
startup path does not select the workspace database. It batches authentication,
optional Sentinel ROLE verification and the allowed read-only operation, and its
Cluster seed fallback may try another seed after a startup transport failure.
Neither behavior is a safe generic write startup contract. The Pub/Sub worker
also translates cancellation into simply “Stopped”, without tracking whether a
mutating command was submitted or its reply was received.

`redis_cluster::Router` routes individual commands and handles MOVED/ASK. It has
no transaction lease pinning a batch and its commit to one node/slot. Reusing this
routing independently for MULTI, queued commands and EXEC would not establish
transaction ownership. Sentinel discovery likewise must finish primary validation
before a dedicated write is submitted; reconnect must never replay that write.

## Release decision

Defer both workflows in this release. The concrete blockers are missing isolated
write lifecycle/cancellation outcome tracking, selected-database startup, and
transaction node ownership. Merely allowing command names or reusing the streaming
worker would weaken existing safeguards. This is not a Redis protocol limitation
or a claim that a future dedicated implementation is impossible. Standalone-only
support would still need the lifecycle and database-selection work.

A future bounded workflow needs a dedicated authenticated, database-selected
session; explicit execution/commit state; cancellation that distinguishes not sent,
confirmed completion and unknown outcome; bounded command/reply counts and bytes;
no automatic write replay; and terminal socket disposal after completion or failure.
Transactions additionally need whole-batch validation, queued/commit result handling,
and explicit supported topology/slot boundaries. Blocking mutations need a finite
server deadline plus socket cancellation with an unknown-outcome warning when the
reply is missing. These are follow-up requirements, not additions to this release.

No production execution path changed during this assessment. Cold/warm exchange
counts, TLS/SSH identity checks, and SQL paths are unchanged. The strengthened
protocol fixture submits each prohibited command after a preceding SET in a batch,
in both read-only modes and lowercase, then proves the peer receives GET as its
first application command. Thus rejection occurs before the earlier SET can run.
Finite blocking timeouts do not bypass the guard. Existing bounded error handling
and connection shutdown assertions run in the same fixture.
