#!/usr/bin/env bash
# One-time (and after-reinstall) setup for the gate's standing databases.
#
# Ruling (Shawn, 2026-09-12): the gate provisions nothing at run time - no
# docker run, no docker pull, no readiness waits inside a gate step. The
# services stand ready on this machine; a gate step that cannot reach one
# fails immediately with instructions, and this script is those instructions.
#
# What the gate uses:
#   MariaDB   - the machine's standing service at 127.0.0.1:3306, through the
#               scoped user scripts/setup/gate-mariadb-user.sql creates.
#   Postgres  - the machine's standing service at 127.0.0.1:5432, through the
#               scoped role scripts/setup/gate-postgres-role.sql creates.
#   Redis     - the machine's standing service at 127.0.0.1:6379. Database
#               indexes 9-15 are reserved for gate runs.
#   MySQL 8.4 - a persistent gate-owned container (nothing else on this
#               machine runs MySQL proper), created here, restart
#               unless-stopped, data on a named volume, loopback-only.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

status=0

if docker inspect suprnova-gate-mysql >/dev/null 2>&1; then
    docker start suprnova-gate-mysql >/dev/null 2>&1 || true
    echo "mysql 8.4:  container suprnova-gate-mysql present"
else
    echo "mysql 8.4:  creating persistent container suprnova-gate-mysql on 127.0.0.1:3317"
    docker run -d --restart unless-stopped --name suprnova-gate-mysql \
        -e MYSQL_ROOT_PASSWORD=suprnova-gate \
        -p 127.0.0.1:3317:3306 \
        -v suprnova-gate-mysql-data:/var/lib/mysql \
        mysql:8.4 >/dev/null
fi
for _ in $(seq 1 90); do
    if mysql -h 127.0.0.1 -P 3317 -u root -psuprnova-gate -e 'select 1' >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
if ! mysql -h 127.0.0.1 -P 3317 -u root -psuprnova-gate -e 'select 1' >/dev/null 2>&1; then
    echo "mysql 8.4:  NOT READY - inspect: docker logs suprnova-gate-mysql" >&2
    status=1
fi

if redis-cli -h 127.0.0.1 -p 6379 ping >/dev/null 2>&1; then
    echo "redis:      standing service answers"
else
    echo "redis:      NOT ANSWERING at 127.0.0.1:6379 - start the host service" >&2
    status=1
fi

if mariadb -h 127.0.0.1 -P 3306 -u suprnova_gate -psuprnova-gate -e 'select 1' >/dev/null 2>&1; then
    echo "mariadb:    standing service answers as suprnova_gate"
else
    echo "mariadb:    gate user missing or service down. Run:" >&2
    echo "            sudo mariadb < scripts/setup/gate-mariadb-user.sql" >&2
    status=1
fi

if PGPASSWORD=suprnova-gate psql -h 127.0.0.1 -p 5432 -U suprnova_gate -d postgres -qAtc 'select 1' >/dev/null 2>&1; then
    echo "postgres:   standing service answers as suprnova_gate"
else
    echo "postgres:   gate role missing or service down. Run:" >&2
    echo "            sudo -u postgres psql -f scripts/setup/gate-postgres-role.sql" >&2
    status=1
fi

exit "$status"
