-- A profile's appearance (theme preset, accent, shape, type, density, Home
-- rows, Now Playing layout, sidebar), shared by the web, desktop and phone
-- apps. The JSON is normalised by the server before it is stored, and
-- `revision` lets each app notice a change made on another device.
CREATE TABLE user_appearance (
  user_id INTEGER NOT NULL PRIMARY KEY,
  settings_json TEXT NOT NULL,
  revision INTEGER NOT NULL DEFAULT 1,
  updated_at BIGINT NOT NULL,
  CONSTRAINT user_appearance_user_fkey
    FOREIGN KEY (user_id) REFERENCES user(id) ON DELETE CASCADE
) WITHOUT ROWID;
