CREATE VIRTUAL TABLE knowledge_fts USING fts5(
    repository_id UNINDEXED,
    entity_kind UNINDEXED,
    entity_id UNINDEXED,
    title,
    body,
    tokenize = 'unicode61'
);
