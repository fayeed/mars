-- A task's conversation is a sequence of runs: the first one starts the
-- provider's session, and each follow-up resumes it. Resuming needs the
-- provider's own session id, which until now was kept only on the raw event
-- stream and was lost as soon as the screen that read it went away.
--
-- `resumes_run_id` links a follow-up to the run whose session it continued, so
-- the chain can be read back without guessing from timestamps. `usage_json` is
-- what the provider reported the run cost — tokens, and dollars where it says —
-- so an overview of many agents can show where the spend went.
ALTER TABLE agent_runs ADD COLUMN provider_session_id TEXT;
ALTER TABLE agent_runs ADD COLUMN resumes_run_id TEXT REFERENCES agent_runs(id) ON DELETE SET NULL;
ALTER TABLE agent_runs ADD COLUMN usage_json TEXT CHECK (usage_json IS NULL OR json_valid(usage_json));
