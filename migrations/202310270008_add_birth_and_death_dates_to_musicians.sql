-- Add migration script here
ALTER TABLE musicians
ADD COLUMN birth_date DATE,
ADD COLUMN death_date DATE;