Released the shared router regex-cache lock before evaluating message matches, avoiding unnecessary serialization across routing requests. (#7604) (@houko)
