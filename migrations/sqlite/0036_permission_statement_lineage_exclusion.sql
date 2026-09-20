-- Migration 0036 — permission statements, derivative lineage, and exclusion registry (M27).
--
-- Dialect: PostgreSQL.
--

-- Add permission statement to works and accounts
ALTER TABLE works ADD COLUMN permission_statement TEXT NOT NULL DEFAULT 'unstated';
ALTER TABLE accounts ADD COLUMN permission_statement TEXT NOT NULL DEFAULT 'unstated';

-- Derivative lineage table
CREATE TABLE derivative_lineage (
    id UUID PRIMARY KEY,
    from_work_id UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    to_work_id UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('translation', 'podfic', 'remix', 'continuation', 'inspired_by')),
    provenance TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- Exclusion registry table
CREATE TABLE exclusion_registry (
    id UUID PRIMARY KEY,
    target_type TEXT NOT NULL CHECK (target_type IN ('work', 'creator')),
    target_id UUID NOT NULL,
    reason TEXT NOT NULL,
    created_by UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL
);

-- Indexes for performance
CREATE INDEX derivative_lineage_from_work ON derivative_lineage(from_work_id);
CREATE INDEX derivative_lineage_to_work ON derivative_lineage(to_work_id);
CREATE INDEX exclusion_registry_target ON exclusion_registry(target_type, target_id);
CREATE INDEX exclusion_registry_created_by ON exclusion_registry(created_by);
