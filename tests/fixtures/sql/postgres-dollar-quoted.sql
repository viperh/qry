-- PostgreSQL: dollar-quoted bodies hold `;` without ending the statement.
create function add_one(i integer) returns integer as $$
begin
  return i + 1; -- a ; in a body
end;
$$ language plpgsql;

create function tagged() returns text as $body$
  select 'it''s $$ not the end;'::text;
$body$ language sql;

select E'escaped \' quote; still a string', "quoted ""ident""" from t;
/* a block comment; with /* nesting */ still a comment */
select 1;
