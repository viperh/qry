-- MySQL: DELIMITER changes the terminator and is not part of a statement.
DELIMITER $$
CREATE PROCEDURE p()
BEGIN
  SELECT `back;tick` FROM t WHERE a = 'x;y';
  SELECT 2;
END$$
DELIMITER ;
select 1; # a hash comment; still a comment
select "double; quoted" from dual;
