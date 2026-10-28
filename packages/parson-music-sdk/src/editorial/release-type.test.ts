import { describe, expect, test } from "bun:test";
import { releaseTypeLabel } from "./release-type";

describe("releaseTypeLabel", () => {
  test("shows a named release type", () => {
    expect(
      releaseTypeLabel({ release_type: { value: "ep", source: "tags" } }),
    ).toBe("EP");
    expect(
      releaseTypeLabel({
        release_type: { value: "ALBUM", source: "musicbrainz" },
      }),
    ).toBe("Album");
    expect(
      releaseTypeLabel({
        release_type: { value: "album + live", source: "edited" },
      }),
    ).toBe("Album + Live");
  });

  test("shows nothing when no tag, match or edit names one", () => {
    // Goldberg Variations and Pink Moon without tags or a match.
    expect(releaseTypeLabel({ release_type: null })).toBe(null);
    expect(
      releaseTypeLabel({ release_type: { value: "  ", source: "tags" } }),
    ).toBe(null);
    expect(releaseTypeLabel(undefined)).toBe(null);
  });
});
