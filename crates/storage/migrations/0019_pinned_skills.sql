-- An installation is a pin: the files and instructions a person trusted, at
-- the version they trusted them. Until now the instructions were read from
-- the skill's row, which inspecting a newer version overwrote — so looking at
-- an update changed what every existing installation said, and reset its
-- trust as a side effect. Each installation now keeps the `SKILL.md` it was
-- installed with; the skill's row describes the latest version inspected.
ALTER TABLE skill_installations ADD COLUMN skill_markdown TEXT NOT NULL DEFAULT '';

UPDATE skill_installations
SET skill_markdown = (SELECT s.skill_markdown FROM skills s WHERE s.id = skill_installations.skill_id);

-- An installation is identified by its skill, scope and target. The original
-- UNIQUE constraint could not say so for global and project installations:
-- SQLite treats NULLs as distinct, so reinstalling a global skill added a
-- second row instead of updating the first. Duplicates are folded into the
-- most recent before the identity is enforced.
DELETE FROM skill_installations
WHERE id IN (
    SELECT older.id FROM skill_installations older
    JOIN skill_installations newer
      ON newer.skill_id = older.skill_id
     AND newer.scope = older.scope
     AND newer.repository_id IS older.repository_id
     AND newer.task_id IS older.task_id
     AND (newer.installed_at > older.installed_at
          OR (newer.installed_at = older.installed_at AND newer.id > older.id))
);

CREATE UNIQUE INDEX skill_installations_identity
    ON skill_installations(skill_id, scope, IFNULL(repository_id, ''), IFNULL(task_id, ''));
