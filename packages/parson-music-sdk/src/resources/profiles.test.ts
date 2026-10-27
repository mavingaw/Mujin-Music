import { afterEach, expect, mock, spyOn, test } from "bun:test";
import api, { ApiError } from "../core/http";
import {
  createProfile,
  deleteProfile,
  getProfiles,
  profileGradient,
  profileInitials,
  switchProfile,
  switcherProfiles,
  updateProfile,
  validProfilePin,
  type Profile,
} from "./profiles";

afterEach(() => {
  mock.restore();
});

const ok = (data: unknown) =>
  ({ data, status: 200, headers: new Headers() }) as never;

const profile = (overrides: Partial<Profile>): Profile => ({
  id: 1,
  name: "Ada",
  username: "ada",
  kind: "account",
  role: "user",
  color: "violet",
  hasPin: false,
  quickSwitch: true,
  isCurrent: false,
  canEdit: false,
  canDelete: false,
  unlock: "none",
  listens: 0,
  scrobbling: [],
  ...overrides,
});

test("profiles use the profile endpoints", async () => {
  const get = spyOn(api, "get").mockResolvedValue(
    ok({ currentId: 1, canCreate: true, colors: [], profiles: [] }),
  );
  await getProfiles();
  expect(get).toHaveBeenCalledWith("/profiles");

  const post = spyOn(api, "post").mockResolvedValue(ok(profile({ id: 4 })));
  await createProfile({ name: "Mia", color: "rose" });
  expect(post).toHaveBeenCalledWith("/profiles", {
    name: "Mia",
    color: "rose",
  });

  const patch = spyOn(api, "patch").mockResolvedValue(ok(profile({})));
  await updateProfile(4, { pin: null });
  expect(patch).toHaveBeenCalledWith("/profiles/4", { pin: null });

  const remove = spyOn(api, "delete").mockResolvedValue(ok(null));
  await deleteProfile(4);
  expect(remove).toHaveBeenCalledWith("/profiles/4");
});

test("switching asks native clients' tokens in the body", async () => {
  const post = spyOn(api, "post").mockResolvedValue(
    ok({ status: true, access_token: "a", refresh_token: "r" }),
  );
  const result = await switchProfile(3, { pin: "2468" }, { native: true });
  expect(result.status).toBeTrue();
  expect(post).toHaveBeenCalledWith(
    "/profiles/3/switch",
    { pin: "2468" },
    { headers: { "X-Parson-Client": "native" } },
  );
  await switchProfile(2);
  expect(post).toHaveBeenLastCalledWith("/profiles/2/switch", {}, undefined);
});

test("a wrong PIN comes back as a message, not an exception", async () => {
  spyOn(api, "post").mockRejectedValue(
    new ApiError("Request failed", {}, {
      data: {
        message: "That PIN is not right.",
        code: "profile_pin_incorrect",
      },
      status: 403,
      headers: new Headers(),
    } as never),
  );
  const result = await switchProfile(1, { pin: "0000" });
  expect(result).toEqual({
    status: false,
    message: "That PIN is not right.",
    transient: false,
  });
});

test("avatar helpers", () => {
  expect(profileInitials("Mia")).toBe("M");
  expect(profileInitials("ada lovelace")).toBe("AL");
  expect(profileInitials("  ")).toBe("?");
  expect(profileInitials("Émile-Zola")).toBe("ÉZ");
  expect(profileGradient("rose")[0]).toBe("#fda4af");
  expect(profileGradient("unknown")).toEqual(profileGradient("slate"));
  expect(validProfilePin("2468")).toBeTrue();
  expect(validProfilePin("246")).toBeFalse();
  expect(validProfilePin("24a8")).toBeFalse();
});

test("the switcher shows the current profile first and hides hidden accounts", () => {
  const ordered = switcherProfiles({
    currentId: 3,
    canCreate: true,
    colors: [],
    profiles: [
      profile({ id: 1, name: "Dad" }),
      profile({ id: 2, name: "Hidden", quickSwitch: false }),
      profile({ id: 3, name: "Sam", isCurrent: true, quickSwitch: false }),
      profile({ id: 4, name: "Mia", kind: "profile" }),
    ],
  });
  expect(ordered.map((item) => item.name)).toEqual(["Sam", "Dad", "Mia"]);
});
