-- Add the column, initially nullable to allow backfilling
ALTER TABLE recordings ADD COLUMN slug_id INTEGER;

-- Backfill existing recordings with a per-artist sequence number
-- We use a CTE to calculate the row number partitioned by artist and ordered by creation date
WITH calculated_slugs AS (
    SELECT id, ROW_NUMBER() OVER (PARTITION BY artist_id ORDER BY created_at ASC) as rn
    FROM recordings
)
UPDATE recordings
SET slug_id = calculated_slugs.rn
FROM calculated_slugs
WHERE recordings.id = calculated_slugs.id;

-- Now make it required
ALTER TABLE recordings ALTER COLUMN slug_id SET NOT NULL;

-- Ensure unique combinations of artist and their slug_id
ALTER TABLE recordings ADD CONSTRAINT uq_recordings_artist_slug_id UNIQUE (artist_id, slug_id);

-- Create a function to auto-increment the slug_id on insert
CREATE OR REPLACE FUNCTION set_recording_slug_id()
RETURNS TRIGGER AS $$
BEGIN
    -- If slug_id is already set (manual override), do nothing
    IF NEW.slug_id IS NOT NULL THEN
        RETURN NEW;
    END IF;

    -- Calculate the next ID for this specific artist
    NEW.slug_id := COALESCE(
        (SELECT MAX(slug_id) FROM recordings WHERE artist_id = NEW.artist_id),
        0
    ) + 1;
    
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Attach the trigger to the table
CREATE TRIGGER trigger_set_recording_slug_id
BEFORE INSERT ON recordings
FOR EACH ROW
EXECUTE FUNCTION set_recording_slug_id();
