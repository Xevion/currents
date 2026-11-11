import { expect, test } from "bun:test";
import { remembered, remember } from "./remember.js";

function memoryStorage() {
  const map = new Map();
  return { getItem: (k) => map.get(k) ?? null, setItem: (k, v) => map.set(k, String(v)), removeItem: (k) => map.delete(k) };
}

test("recalls a fallback for the same browser", () => {
  const storage = memoryStorage();
  remember(storage, "UA 1", "webgl", ["webgpu", "webgl"]);
  expect(remembered(storage, "UA 1")).toBe("webgl");
});

test("forgets it once the browser changes, since an update may have fixed the first choice", () => {
  const storage = memoryStorage();
  remember(storage, "UA 1", "webgl", ["webgpu", "webgl"]);
  expect(remembered(storage, "UA 2")).toBeNull();
});

test("clears it when the first choice works again", () => {
  const storage = memoryStorage();
  remember(storage, "UA 1", "webgl", ["webgpu", "webgl"]);
  remember(storage, "UA 1", "webgpu", ["webgpu", "webgl"]);
  expect(remembered(storage, "UA 1")).toBeNull();
});

test("survives storage that throws or holds junk", () => {
  const throwing = { getItem() { throw new Error("denied"); }, setItem() { throw new Error("denied"); }, removeItem() {} };
  expect(remembered(throwing, "UA")).toBeNull();
  expect(() => remember(throwing, "UA", "webgl", ["webgpu", "webgl"])).not.toThrow();
  const junk = memoryStorage();
  junk.setItem("currents.backend", "{not json");
  expect(remembered(junk, "UA")).toBeNull();
});
