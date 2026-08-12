Fixed the Python sidecar's Telegram HTML sanitizer emitting invalid crossed tags (e.g. `<b><i>x</b>` → `<b><i>x</b></i>`) when a closing tag matched an entry below the top of the open-tag stack.
The sanitizer now closes every tag above (and including) the match, innermost first, matching the Rust sanitizer's stack-drain behavior. (#6856) (@houko)
