CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT);INSERT INTO replacement SELECT * FROM users;DROP TABLE users;ALTER TABLE replacement RENAME TO users;
