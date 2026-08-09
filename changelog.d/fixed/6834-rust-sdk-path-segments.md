Encode generated Rust SDK path parameters as URL segments. (@houko)
Generated endpoints previously interpolated path values directly, so slashes, query/fragment delimiters, whitespace, and Unicode could change the request target or address a different resource.
URLs are now assembled with `Url::path_segments_mut`, preserving base-path prefixes and percent-encoding each parameter as one segment; literal `.` and `..` segments fail closed instead of being normalized away.
