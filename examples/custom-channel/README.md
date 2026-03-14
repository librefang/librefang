# Custom Channel Adapter Example

This example demonstrates how to implement a custom channel adapter for LibreFang.

## Overview

Channel adapters bridge external messaging platforms to the LibreFang kernel. Every adapter implements the `ChannelAdapter` trait defined in `crates/librefang-channels/src/types.rs`.

For full documentation, see [docs/channel-adapters.md](../../docs/channel-adapters.md).

## The `ChannelAdapter` Trait

The core trait requires four methods and provides several optional ones:

| Method | Required | Description |
|--------|----------|-------------|
| `name()` | Yes | Human-readable adapter name |
| `channel_type()` | Yes | Returns a `ChannelType` variant |
| `start()` | Yes | Begin receiving messages; returns a `Stream<Item = ChannelMessage>` |
| `send()` | Yes | Send a response to a user |
| `stop()` | Yes | Clean shutdown |
| `send_typing()` | No | Send a typing indicator (default: no-op) |
| `send_reaction()` | No | Send a lifecycle reaction emoji (default: no-op) |
| `status()` | No | Report adapter health (default: disconnected) |
| `send_in_thread()` | No | Reply in a thread (default: falls back to `send()`) |

## Minimal Example

See `adapter.rs` in this directory for a minimal implementation that polls a directory for `.txt` files as incoming messages and writes responses to an output directory.

## Integration Steps

After implementing the trait:

1. Add your module to `crates/librefang-channels/src/lib.rs`
2. Wire it into the channel bridge in `crates/librefang-api/src/channel_bridge.rs`
3. Add config support in `librefang-types` config structs
4. Write tests
5. Submit a PR

See [CONTRIBUTING.md](../../CONTRIBUTING.md#how-to-add-a-new-channel-adapter) for the full walkthrough.
