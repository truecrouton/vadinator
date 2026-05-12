CREATE TABLE context (
	context_id INTEGER PRIMARY KEY,
	system_prompt TEXT NOT NULL,
	deactivated INTEGER DEFAULT 0,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);