-- SQLite and friends: brackets, blobs, unicode and an unterminated string.
create table "t ü" ([a b] integer, c blob);
insert into "t ü" values (1, x'00ff'), (2, '🦀; crab');
select * from "t ü" where c is not null;
select 'never closed;
