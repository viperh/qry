#!/bin/sh
# Prints the variables the database integration suite (T004) reads, for
# the servers of deploy/docker-compose.test.yml:
#
#   eval "$(scripts/db-test-env.sh)"
#
# QRY_TEST_HOST overrides the host (default: localhost).
set -eu

host="${QRY_TEST_HOST:-localhost}"

cat <<EOF
export QRY_TEST_POSTGRES_URL='postgres://qry:qry@${host}:5432/qry'
export QRY_TEST_MYSQL_URL='mysql://root:qry@${host}:3306/qry'
export QRY_TEST_MARIADB_URL='mysql://root:qry@${host}:3307/qry'
export QRY_TEST_ORACLE_URL='oracle://system:qry@${host}:1521/FREEPDB1'
EOF
