-- The database integration suite's seed (T004), in MySQL; MariaDB loads it
-- too. The same schema and rows exist for every backend: seed_postgres.sql,
-- seed_oracle.sql and seed_sqlite.sql. Change them together.

CREATE TABLE people (
    id      int PRIMARY KEY,
    name    text NOT NULL,
    born    date,
    balance decimal(12, 2),
    active  bool,
    avatar  blob,
    note    text NULL
) DEFAULT CHARSET = utf8mb4;

INSERT INTO people VALUES (1, 'Ada', DATE '1990-04-01', -12.50, TRUE, X'00ff10', 'first');
INSERT INTO people VALUES (2, 'Zoë', DATE '1985-12-31', 1000.00, FALSE, NULL, '');
INSERT INTO people VALUES (3, '東京', NULL, 0.00, TRUE, X'deadbeef', NULL);
INSERT INTO people VALUES (4, 'Bob', DATE '2000-02-29', 42.10, FALSE, NULL, 'likes ''quotes''');
INSERT INTO people VALUES (5, 'Eve', DATE '1970-01-01', NULL, NULL, NULL, NULL);

CREATE VIEW active_people AS SELECT id, name FROM people WHERE active = TRUE;

CREATE TABLE orders (
    id        int PRIMARY KEY,
    person_id int NOT NULL,
    total     decimal(12, 2) NOT NULL,
    FOREIGN KEY (person_id) REFERENCES people (id)
) DEFAULT CHARSET = utf8mb4;

INSERT INTO orders VALUES (1, 1, 9.99);
INSERT INTO orders VALUES (2, 1, 20.00);
INSERT INTO orders VALUES (3, 3, 5.25);
