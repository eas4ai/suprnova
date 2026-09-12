-- One-time: scoped gate user for Suprnova's local gate. May create, use, and
-- drop only databases under the gate's two prefix namespaces - suprnova_*
-- and magnetar_* - which covers the per-run gate databases (suprnova_test_*,
-- magnetar_test_*) and the isolated fixture databases the Magnetar upgrade
-- tests create themselves (magnetar_upgrade_mysql_*). Re-runnable.
CREATE USER IF NOT EXISTS 'suprnova_gate'@'localhost' IDENTIFIED BY 'suprnova-gate';
CREATE USER IF NOT EXISTS 'suprnova_gate'@'127.0.0.1' IDENTIFIED BY 'suprnova-gate';
GRANT ALL PRIVILEGES ON `suprnova\_%`.* TO 'suprnova_gate'@'localhost';
GRANT ALL PRIVILEGES ON `suprnova\_%`.* TO 'suprnova_gate'@'127.0.0.1';
GRANT ALL PRIVILEGES ON `magnetar\_%`.* TO 'suprnova_gate'@'localhost';
GRANT ALL PRIVILEGES ON `magnetar\_%`.* TO 'suprnova_gate'@'127.0.0.1';
FLUSH PRIVILEGES;
