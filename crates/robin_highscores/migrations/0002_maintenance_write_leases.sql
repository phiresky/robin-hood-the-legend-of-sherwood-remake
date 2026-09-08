CREATE TABLE maintenance_write_leases(
  token TEXT PRIMARY KEY NOT NULL CHECK(length(token) BETWEEN 20 AND 64),
  writer_class TEXT NOT NULL CHECK(writer_class IN(
    'api_sensitive',
    'api_upload',
    'api_maintenance',
    'worker',
    'admin'
  )),
  owner TEXT NOT NULL CHECK(length(owner) BETWEEN 1 AND 128),
  acquired_at_ms INTEGER NOT NULL CHECK(acquired_at_ms >= 0),
  expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms > acquired_at_ms)
) STRICT;

CREATE INDEX maintenance_write_leases_expiry_idx
ON maintenance_write_leases(expires_at_ms);

CREATE INDEX maintenance_write_leases_class_expiry_idx
ON maintenance_write_leases(writer_class, expires_at_ms);
