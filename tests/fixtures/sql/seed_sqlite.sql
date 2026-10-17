-- The database integration suite's seed (T004), in SQLite. The same schema
-- and rows exist for every backend: seed_postgres.sql, seed_mysql.sql and
-- seed_oracle.sql. Change them together.
--
-- SQLite types values, not columns: `numeric(12, 2)` stores -12.50 as the
-- REAL -12.5, and `boolean` stores TRUE as the integer 1. The contract
-- tests spell out those differences.

CREATE TABLE people (
    id      integer PRIMARY KEY,
    name    text NOT NULL,
    born    date,
    balance numeric(12, 2),
    active  boolean,
    avatar  blob,
    note    text NULL
);

INSERT INTO people VALUES (1, 'Ada', '1990-04-01', -12.50, TRUE, X'00ff10', 'first');
INSERT INTO people VALUES (2, 'Zoë', '1985-12-31', 1000.00, FALSE, NULL, '');
INSERT INTO people VALUES (3, '東京', NULL, 0.00, TRUE, X'deadbeef', NULL);
INSERT INTO people VALUES (4, 'Bob', '2000-02-29', 42.10, FALSE, NULL, 'likes ''quotes''');
INSERT INTO people VALUES (5, 'Eve', '1970-01-01', NULL, NULL, NULL, NULL);

CREATE VIEW active_people AS SELECT id, name FROM people WHERE active = TRUE;

CREATE TABLE orders (
    id        integer PRIMARY KEY,
    person_id integer NOT NULL REFERENCES people (id),
    total     numeric(12, 2) NOT NULL
);

INSERT INTO orders VALUES (1, 1, 9.99);
INSERT INTO orders VALUES (2, 1, 20.00);
INSERT INTO orders VALUES (3, 3, 5.25);
