CREATE TABLE ica_spending (
  id serial NOT NULL,
  store character varying(100) NULL,
  discount real NULL,
  transaction real NULL,
  date date NULL,
  transaction_id character varying(12) NOT NULL
);
