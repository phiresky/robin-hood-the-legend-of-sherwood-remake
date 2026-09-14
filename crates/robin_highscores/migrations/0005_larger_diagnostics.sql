-- Raise the compressed report budget while retaining every existing payload.
ALTER TABLE diagnostic_reports RENAME TO diagnostic_reports_v4;
DROP INDEX diagnostic_reports_received;
DROP INDEX diagnostic_reports_ip;
CREATE TABLE diagnostic_reports (
    id TEXT PRIMARY KEY NOT NULL,
    received_at_ms INTEGER NOT NULL,
    reporter_ip_hash BLOB NOT NULL CHECK(length(reporter_ip_hash) = 32),
    payload BLOB NOT NULL,
    payload_bytes INTEGER NOT NULL CHECK(payload_bytes > 0 AND payload_bytes <= 104857600),
    encoding TEXT NOT NULL CHECK(encoding IN ('identity', 'zstd', 'gzip')),
    kind TEXT NOT NULL,
    engine_commit TEXT NOT NULL
);
INSERT INTO diagnostic_reports SELECT * FROM diagnostic_reports_v4;
DROP TABLE diagnostic_reports_v4;
CREATE INDEX diagnostic_reports_received ON diagnostic_reports(received_at_ms);
CREATE INDEX diagnostic_reports_ip ON diagnostic_reports(reporter_ip_hash, received_at_ms);
