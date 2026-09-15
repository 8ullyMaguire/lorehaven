-- M14 — Governance: trust, reports, quorum, appeals, sanctions

CREATE TABLE trust_levels (
    account TEXT PRIMARY KEY,
    level INTEGER NOT NULL,
    computed_at TEXT NOT NULL,
    basis TEXT NOT NULL
);

CREATE TABLE reports (
    id TEXT PRIMARY KEY,
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    reporter TEXT NOT NULL,
    reason TEXT NOT NULL,
    created_at TEXT NOT NULL,
    state TEXT NOT NULL,
    resolved_at TEXT,
    resolution TEXT NOT NULL
);
CREATE INDEX idx_reports_state_created ON reports(state, created_at);

CREATE TABLE review_tasks (
    id TEXT PRIMARY KEY,
    reviewer TEXT NOT NULL,
    report_id TEXT NOT NULL,
    assigned_at TEXT NOT NULL,
    decided_at TEXT,
    outcome TEXT NOT NULL
);
CREATE INDEX idx_review_tasks_report ON review_tasks(report_id);

CREATE TABLE sanctions (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    kind TEXT NOT NULL,
    reason_ref TEXT NOT NULL,
    starts_at TEXT NOT NULL,
    ends_at TEXT,
    issued_by TEXT NOT NULL,
    lifted_at TEXT,
    lifted_by TEXT
);
CREATE INDEX idx_sanctions_account_starts ON sanctions(account, starts_at);

CREATE TABLE appeals (
    id TEXT PRIMARY KEY,
    sanction_id TEXT NOT NULL,
    appellant TEXT NOT NULL,
    statement TEXT NOT NULL,
    created_at TEXT NOT NULL,
    state TEXT NOT NULL,
    decided_at TEXT,
    decision TEXT NOT NULL,
    decided_by TEXT NOT NULL
);

CREATE TABLE audit_log (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    document JSONB NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_audit_subject ON audit_log(subject_type, subject_id, created_at);

CREATE TABLE operator_role (
    account TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    granted_at TEXT NOT NULL
);

CREATE TABLE preservation_batches (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    query TEXT NOT NULL,
    destination TEXT NOT NULL,
    dry_run BOOLEAN NOT NULL,
    approval_basis TEXT NOT NULL,
    approved_by TEXT NOT NULL,
    ran_at TEXT,
    summary JSONB
);
