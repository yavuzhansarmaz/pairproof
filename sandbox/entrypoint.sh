#!/bin/sh
set -eu
mkdir -p /run/sshd /workspace /run/mysqld
chown mysql:mysql /run/mysqld
ssh-keygen -A
if [ ! -d /var/lib/mysql/mysql ]; then
    mariadb-install-db --user=mysql --datadir=/var/lib/mysql --skip-test-db >/dev/null 2>&1 || true
fi
(mariadbd --user=mysql --datadir=/var/lib/mysql --socket=/run/mysqld/mysqld.sock --skip-networking --pid-file=/run/mysqld/mysqld.pid >/var/log/mariadbd.log 2>&1 &)
i=0
while [ "$i" -lt 20 ]; do
    if [ -S /run/mysqld/mysqld.sock ]; then
        break
    fi
    i=$((i + 1))
    sleep 1
done
exec /usr/sbin/sshd -D -e
