import { expect, test } from "bun:test";
import { isNavItemActive, sidebarSections } from "./sidebar-navigation";

const item = (href: string) => ({ href, label: href, icon: "x" });

test("library views are active only for their own view", () => {
  const albums = item("/library");
  const artists = item("/library?view=artists");
  expect(isNavItemActive(albums, { pathname: "/library", view: null })).toBe(
    true,
  );
  expect(
    isNavItemActive(albums, { pathname: "/library", view: "artists" }),
  ).toBe(false);
  expect(
    isNavItemActive(artists, { pathname: "/library", view: "artists" }),
  ).toBe(true);
  expect(isNavItemActive(artists, { pathname: "/album", view: null })).toBe(
    false,
  );
});

test("home matches only itself; other items match nested routes", () => {
  expect(isNavItemActive(item("/"), { pathname: "/", view: null })).toBe(true);
  expect(isNavItemActive(item("/"), { pathname: "/genre", view: null })).toBe(
    false,
  );
  expect(
    isNavItemActive(item("/radio"), { pathname: "/radio/station", view: null }),
  ).toBe(true);
  expect(
    isNavItemActive(item("/radio"), { pathname: "/radiology", view: null }),
  ).toBe(false);
});

test("the sidebar keeps Roon's Browse and My Library groups", () => {
  expect(sidebarSections.map((section) => section.title)).toEqual([
    "Browse",
    "My Library",
  ]);
  const hrefs = sidebarSections.flatMap((section) =>
    section.items.map((entry) => entry.href),
  );
  expect(new Set(hrefs).size).toBe(hrefs.length);
});
