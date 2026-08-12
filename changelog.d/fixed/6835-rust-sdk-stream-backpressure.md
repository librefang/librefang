Bound generated Rust SDK stream buffering to 256 events. (@houko)
Streaming previously used Tokio's unbounded channel, allowing a fast server and stalled consumer to grow memory without limit.
The producer now awaits a bounded channel, applying transport backpressure and stopping promptly when the receiver is dropped. Stream methods consequently return `tokio::sync::mpsc::Receiver<Value>`; callers with explicit `UnboundedReceiver` annotations must update the annotation, while normal inferred `.recv()` usage is unchanged.
