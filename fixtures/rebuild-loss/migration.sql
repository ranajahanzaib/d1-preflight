CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT);INSERT INTO replacement SELECT * FROM users WHERE id=1;DROP TABLE users;ALTER TABLE replacement RENAME TO users;
