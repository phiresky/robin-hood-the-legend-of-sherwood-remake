-- Preserve queued diagnostics from schema 3 without rewriting migration history.
ALTER TABLE diagnostic_reports RENAME TO diagnostic_reports_v3;
DROP INDEX diagnostic_reports_received;
DROP INDEX diagnostic_reports_ip;
CREATE TABLE diagnostic_reports (
    id TEXT PRIMARY KEY NOT NULL,
    received_at_ms INTEGER NOT NULL,
    reporter_ip_hash BLOB NOT NULL CHECK(length(reporter_ip_hash) = 32),
    payload BLOB NOT NULL,
    payload_bytes INTEGER NOT NULL CHECK(payload_bytes > 0 AND payload_bytes <= 20971520),
    encoding TEXT NOT NULL CHECK(encoding IN ('identity', 'zstd', 'gzip')),
    kind TEXT NOT NULL,
    engine_commit TEXT NOT NULL
);
INSERT INTO diagnostic_reports
SELECT id, received_at_ms, reporter_ip_hash, CAST(payload AS BLOB), payload_bytes,
       'identity', json_extract(payload, '$.kind'), json_extract(payload, '$.engine_commit')
FROM diagnostic_reports_v3;
DROP TABLE diagnostic_reports_v3;
CREATE INDEX diagnostic_reports_received ON diagnostic_reports(received_at_ms);
CREATE INDEX diagnostic_reports_ip ON diagnostic_reports(reporter_ip_hash, received_at_ms);
