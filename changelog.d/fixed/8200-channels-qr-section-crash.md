Opening a second channel's details drawer no longer takes the whole dashboard down with it.
The QR section stops polling once the daemon answers 204 or 404 for a channel that does not use QR login, and that latch is component state, so it outlived a switch to another channel: the effect that clears it runs only after the render that already saw the new channel name.
That one render asked for a disabled query on a cache key nothing had fetched, whose `data` is `undefined` rather than the 204's `null`, and the section — guarded only against loading, error and `null` — dereferenced it and threw during render.
The only boundary above it is the root one, so a `TypeError` reading a QR status replaced the entire app shell until the operator reloaded.
The section now hides itself whenever it has no QR projection to draw, `undefined` included (#8200) (@houko)
