-- The database integration suite's seed (T004), in Oracle (23ai: BOOLEAN
-- is a column type). The same schema and rows exist for every backend:
-- seed_postgres.sql, seed_mysql.sql and seed_sqlite.sql. Change them
-- together.
--
-- Oracle stores '' as NULL, so person 2's note is NULL here; the contract
-- test `null_is_none_and_empty_string_is_some_empty` says so explicitly.

CREATE TABLE people (
    id      NUMBER(10) PRIMARY KEY,
    name    VARCHAR2(100) NOT NULL,
    born    DATE,
    balance NUMBER(12, 2),
    active  BOOLEAN,
    avatar  RAW(16),
    note    CLOB
);

INSERT INTO people VALUES (1, 'Ada', DATE '1990-04-01', -12.50, TRUE, HEXTORAW('00ff10'), 'first');
INSERT INTO people VALUES (2, 'Zoë', DATE '1985-12-31', 1000.00, FALSE, NULL, '');
INSERT INTO people VALUES (3, '東京', NULL, 0.00, TRUE, HEXTORAW('deadbeef'), NULL);
INSERT INTO people VALUES (4, 'Bob', DATE '2000-02-29', 42.10, FALSE, NULL, 'likes ''quotes''');
INSERT INTO people VALUES (5, 'Eve', DATE '1970-01-01', NULL, NULL, NULL, NULL);

CREATE VIEW active_people AS SELECT id, name FROM people WHERE active = TRUE;

CREATE TABLE orders (
    id        NUMBER(10) PRIMARY KEY,
    person_id NUMBER(10) NOT NULL REFERENCES people (id),
    total     NUMBER(12, 2) NOT NULL
);

INSERT INTO orders VALUES (1, 1, 9.99);
INSERT INTO orders VALUES (2, 1, 20.00);
INSERT INTO orders VALUES (3, 3, 5.25);
