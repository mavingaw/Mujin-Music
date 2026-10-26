-- The last "Check connection" result for each TIDAL or Qobuz account, as
-- JSON: when it ran, whether it was the sandbox or the real service, and
-- what the stream delivered (format, full length or preview). Settings shows
-- it so a listener sees whether their own account has been proven, not just
-- the recorded fixtures. New credentials clear it.
ALTER TABLE provider_account ADD COLUMN last_check TEXT;
