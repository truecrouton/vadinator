CREATE TABLE message (
	message_id INTEGER PRIMARY KEY,
	role TEXT NOT NULL,
    content TEXT NOT NULL,
    context_id INTEGER NOT NULL,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);