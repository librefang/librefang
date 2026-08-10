Image generation works again against OpenAI's `gpt-image-*` models.
  The request always carried `response_format`, which that family rejects with `400 Unknown parameter`, so every generation against those models failed while DALL-E was unaffected.
  The parameter is now sent only to models that accept it, and unrecognised model names keep the previous behaviour so third-party OpenAI-compatible endpoints are unchanged.
  (#6750) (@houko)
