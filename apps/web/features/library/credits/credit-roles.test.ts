import { expect, test } from "bun:test";
import { creditHref } from "./credit-roles";

test("credit links open the credits tab filtered by role", () => {
  expect(creditHref("mb:abc", "producer")).toBe(
    "/artist?id=mb%3Aabc&tab=credits&role=producer",
  );
  expect(creditHref("x")).toBe("/artist?id=x&tab=credits");
});
