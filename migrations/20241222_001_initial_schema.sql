-- migrations/20241222_001_initial_schema.sql

-- Subjects (users/accounts)
CREATE TABLE subjects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL UNIQUE,
    account_id TEXT,
    kyc_level TEXT NOT NULL DEFAULT 'L0',
    geo_iso TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Addresses linked to subjects
CREATE TABLE subject_addresses (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    chain TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(subject_id, address)
);
CREATE INDEX idx_subject_addresses_address ON subject_addresses(address);

-- Transaction history (for streaming rules)
CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES subjects(id),
    tx_type TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount NUMERIC NOT NULL,
    usd_value NUMERIC NOT NULL,
    dest_address TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_transactions_subject_time ON transactions(subject_id, created_at DESC);

-- Sanctions list
CREATE TABLE sanctions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    address TEXT NOT NULL UNIQUE,
    source TEXT,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Policies (JSONB for flexibility)
CREATE TABLE policies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version TEXT NOT NULL UNIQUE,
    config JSONB NOT NULL,
    active BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_policies_single_active ON policies(active) WHERE active = true;

-- Decision audit log
CREATE TABLE decisions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID REFERENCES subjects(id),
    request JSONB NOT NULL,
    decision TEXT NOT NULL,
    decision_code TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    evidence JSONB,
    latency_ms INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_decisions_subject_time ON decisions(subject_id, created_at DESC);
