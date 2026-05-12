CREATE TABLE stash (
	stash_id INTEGER PRIMARY KEY,
    context_id INTEGER NOT NULL,
    source TEXT NOT NULL,
    source_type TEXT NOT NULL,
    topic TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX stash_topic_idx ON stash(topic);