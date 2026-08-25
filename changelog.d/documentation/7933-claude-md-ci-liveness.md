The agent instructions now state the one honest test for whether CI is executing, and why every cheaper proxy for it reports a live pool against a dead one.
A workflow file that fails to parse completes as a failure within seconds without ever taking a runner, so those runs flicker through the in-progress state and fill the completed list, and a poll on either signal reads a stalled queue as throughput.
The same section records that a `cargo check` finishing in under a second proves nothing in a shared target directory until a `compile_error!` sentinel confirms the compiler actually read the tree.
(#7933) (@houko)
