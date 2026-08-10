The media integration tests no longer depend on the developer's shell lacking provider credentials.
  With an API key exported, tests asserting the missing-key path stopped exercising it and instead made real, billable calls — one generated an mp3 through the live OpenAI TTS endpoint while asserting that no provider was configured.
  The harness now clears every credential variable the media drivers read, and a dedicated test fails if that ever stops happening.
  A unit test that resolved a provider before reading its input file had the same dependency and could have reached a live transcription request; its input path is now guaranteed absent.
  (#6750) (@houko)
