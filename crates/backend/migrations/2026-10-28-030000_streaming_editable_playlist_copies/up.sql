-- Saved TIDAL and Qobuz playlist copies become editable Parson playlists:
-- listeners rename them, reorder, remove and add tracks. `edited_at` marks a
-- copy changed in Parson, so saving the service playlist again asks before
-- replacing those edits. Copies Parson made itself (from a radio or a
-- selection) use ids starting with `parson-` and have no service playlist.
ALTER TABLE provider_playlist_copy ADD COLUMN edited_at BIGINT;
