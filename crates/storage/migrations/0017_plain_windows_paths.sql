-- On Windows, `std::fs::canonicalize` answers in the verbatim form
-- `\\?\C:\Users\...`. Mars stored that form as a repository's identity, and then
-- handed it on: to Git as a working directory, to agent CLIs as their cwd, to
-- the terminal, and to every screen that shows a path. Tools that do not expect
-- the prefix mis-handle it, and a person reading `\\?\C:\` cannot tell what it
-- means or whether it is the folder they chose.
--
-- Canonicalisation now returns the ordinary drive-letter form wherever one
-- exists. This rewrites what was already stored the same way, so a repository
-- opened before keeps its identity rather than being trusted a second time as a
-- stranger. Only drive-letter paths short enough to be represented without the
-- prefix are rewritten: a path the ordinary form cannot express keeps the
-- verbatim one, because that is what canonicalisation still returns for it.
UPDATE repositories
SET root_path = substr(root_path, 5)
WHERE root_path LIKE '\\?\_:\%'
  AND length(root_path) - 4 < 260
  AND NOT EXISTS (
    SELECT 1 FROM repositories other WHERE other.root_path = substr(repositories.root_path, 5)
  );

UPDATE repositories
SET git_common_dir = substr(git_common_dir, 5)
WHERE git_common_dir LIKE '\\?\_:\%'
  AND length(git_common_dir) - 4 < 260;
