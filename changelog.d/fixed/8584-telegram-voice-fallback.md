A Telegram voice reply to a user whose privacy settings forbid voice messages is now delivered as an audio document with its caption instead of vanishing.
`sendVoice` failed with `VOICE_MESSAGES_FORBIDDEN` and the reply was dropped without a trace; both the Python and Rust sidecars now fall back to `sendDocument` and log the refusal (#8587, #8584) (@houko)
