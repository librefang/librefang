-- Migration v29: add model_override field to sessions for per-session model pin.
-- Mirrors upstream SQLite v39 (#4898): ALTER TABLE sessions ADD COLUMN model_override TEXT DEFAULT NULL.
--
-- When set, the kernel's execute_llm_agent uses this value instead of the
-- agent manifest's model fields for LLM dispatch on that session.
-- NULL preserves existing behaviour (agent default). Stored as "<provider>/<model>"
-- when provider is specified, or just "<model>" for provider-agnostic overrides.
-- Set/cleared via PATCH /api/sessions/{id}/model.

DEFINE FIELD IF NOT EXISTS model_override ON sessions TYPE option<string>;
