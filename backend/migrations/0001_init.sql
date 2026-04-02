CREATE TABLE IF NOT EXISTS items (
    id TEXT PRIMARY KEY,
    query TEXT NOT NULL,
    priority TEXT NOT NULL DEFAULT 'immediate',
    blacklisted_brands TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'active',
    todoist_task_id TEXT,
    todoist_section_id TEXT,
    current_ean TEXT,
    current_chain TEXT,
    current_product_name TEXT,
    current_brand TEXT,
    current_price REAL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
