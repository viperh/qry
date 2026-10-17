-- Oracle: a PL/SQL block ends at a line holding only `/`, not at `;`.
create or replace procedure bump(p_id in number) as
  v_count number;
begin
  select count(*) into v_count from orders where customer_id = p_id;
  update customers set order_count = v_count where id = p_id;
  dbms_output.put_line(q'[it's done; really]');
end;
/

declare
  msg varchar2(30) := 'a ; inside a string';
begin
  null;
end;
/

select sysdate from dual;
select q'{braces}' as quoted, "Mixed Case" from t;
