Emit the `[File: …] saved to …` attachment marker ahead of the inlined enrichment body on the channel download path.
An agent keys on that marker to find the raw file on disk, but it was pushed after an inlined body that can reach 200,000 characters, so for a text-like attachment the marker landed at the tail of the message and the agent worked from the truncated inline copy instead of the saved file.
Nothing depended on the old order: `has_file_saved_block`, the marker's only consumer, matches the prefix on any block (#8627) (@nevgenov)
