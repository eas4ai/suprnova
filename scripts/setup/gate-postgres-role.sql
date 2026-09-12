-- One-time: scoped gate role for Suprnova's local gate. CREATEDB lets it
-- create and drop only databases it owns (suprnova_test_*, magnetar_test_*).
CREATE ROLE suprnova_gate LOGIN PASSWORD 'suprnova-gate' CREATEDB;
