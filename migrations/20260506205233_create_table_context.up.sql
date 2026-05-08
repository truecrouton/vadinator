CREATE TABLE context (
	context_id INTEGER PRIMARY KEY,
	context TEXT NOT NULL,
	system_prompt TEXT NOT NULL,
	deactivated INTEGER DEFAULT 0,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX context_idx ON context(context, deactivated);