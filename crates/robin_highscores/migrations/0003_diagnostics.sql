CREATE TABLE diagnostic_reports (
    id TEXT PRIMARY KEY NOT NULL,
    received_at_ms INTEGER NOT NULL,
    reporter_ip_hash BLOB NOT NULL CHECK(length(reporter_ip_hash) = 32),
    payload TEXT NOT NULL,
    payload_bytes INTEGER NOT NULL CHECK(payload_bytes > 0 AND payload_bytes <= 2097152)
);
CREATE INDEX diagnostic_reports_received ON diagnostic_reports(received_at_ms);
CREATE INDEX diagnostic_reports_ip ON diagnostic_reports(reporter_ip_hash, received_at_ms);
