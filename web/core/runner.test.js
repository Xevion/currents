import { describe, expect, test } from "bun:test";
import { launch, notice, order } from "./runner.js";

describe("order", () => {
  test("prefers WebGPU, then WebGL2, among what the browser supports", () => {
    expect(order({ supported: ["webgl", "webgpu"] })).toEqual(["webgpu", "webgl"]);
    expect(order({ supported: ["webgl"] })).toEqual(["webgl"]);
  });

  test("tries a pinned backend alone, even one the probe found unsupported", () => {
    expect(order({ supported: ["webgl"], pinned: "webgpu" })).toEqual(["webgpu"]);
  });

  test("tries the remembered backend first and keeps the rest as fallbacks", () => {
    expect(order({ supported: ["webgpu", "webgl"], remembered: "webgl" })).toEqual(["webgl", "webgpu"]);
  });

  test("ignores a remembered backend the browser no longer supports", () => {
    expect(order({ supported: ["webgpu"], remembered: "webgl" })).toEqual(["webgpu"]);
  });
});

describe("notice", () => {
  test("names what failed and what runs instead", () => {
    const failed = [{ backend: "webgpu", stage: "shaders", message: "bad shader", drawn: false }];
    expect(notice(failed, "webgl")).toBe("WebGPU did not start (bad shader); running on WebGL2 instead.");
  });

  test("says a backend stopped when it had been drawing", () => {
    const failed = [{ backend: "webgpu", stage: "lost", message: "device lost", drawn: true }];
    expect(notice(failed, "webgl")).toBe("WebGPU stopped (device lost); running on WebGL2 instead.");
  });

  test("drops the message's own closing period inside the parentheses", () => {
    const failed = [{ backend: "webgpu", stage: "lost", message: "Device was destroyed.", drawn: true }];
    expect(notice(failed, "webgl")).toBe("WebGPU stopped (Device was destroyed); running on WebGL2 instead.");
  });

  test("is empty when nothing failed or the same backend restarts", () => {
    expect(notice([], "webgpu")).toBeNull();
    expect(notice([{ backend: "webgl", stage: "lost", message: "x", drawn: true }], "webgl")).toBeNull();
  });
});

/** A fake backend start: each backend follows a script of outcomes, one per start. */
function fakeStarts(scripts) {
  const calls = [];
  const sessions = [];
  const start = (backend, options) => {
    calls.push({ backend, notice: options.notice });
    const outcome = scripts[backend].shift();
    if (outcome.fails) {
      return Promise.reject({ backend, stage: outcome.fails, message: outcome.fails, drawn: false });
    }
    let end;
    const session = { backend, stopped: false, stop() { this.stopped = true; } };
    session.ended = new Promise((resolve) => (end = resolve));
    session.end = (stage) => end({ backend, stage, message: stage, drawn: true });
    sessions.push(session);
    return Promise.resolve(session);
  };
  return { start, calls, sessions };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function run(options) {
  const states = [];
  const slept = [];
  let clock = 0;
  const done = launch({
    policy: { restarts: 0 },
    sleep: async (ms) => {
      slept.push(ms);
      clock += ms;
    },
    now: () => clock,
    onState: (state) => states.push(state),
    ...options,
  });
  return { done, states, slept, advance: (ms) => (clock += ms) };
}

describe("launch", () => {
  test("falls back to the next backend when one fails before drawing, telling the next why", async () => {
    const fake = fakeStarts({ webgpu: [{ fails: "device" }], webgl: [{}] });
    const { states } = run({ backends: ["webgpu", "webgl"], start: fake.start });
    await settle();
    expect(fake.calls.map((c) => c.backend)).toEqual(["webgpu", "webgl"]);
    expect(fake.calls[1].notice).toBe("WebGPU did not start (device); running on WebGL2 instead.");
    expect(states.at(-1)).toMatchObject({ phase: "running", backend: "webgl" });
  });

  test("reports every failure once no backend is left", async () => {
    const fake = fakeStarts({ webgpu: [{ fails: "device" }], webgl: [{ fails: "shaders" }] });
    const { done, states } = run({ backends: ["webgpu", "webgl"], start: fake.start });
    await done;
    expect(states.at(-1)).toMatchObject({ phase: "failed" });
    expect(states.at(-1).failures.map((f) => f.stage)).toEqual(["device", "shaders"]);
  });

  test("switches to the next backend when a running one stops, stopping the old session", async () => {
    const fake = fakeStarts({ webgpu: [{}], webgl: [{}] });
    const { states } = run({ backends: ["webgpu", "webgl"], start: fake.start });
    await settle();
    fake.sessions[0].end("lost");
    await settle();
    expect(fake.sessions[0].stopped).toBe(true);
    expect(fake.calls[1]).toEqual({ backend: "webgl", notice: "WebGPU stopped (lost); running on WebGL2 instead." });
    expect(states.at(-1)).toMatchObject({ phase: "running", backend: "webgl" });
  });

  test("never switches away from a pinned backend", async () => {
    const fake = fakeStarts({ webgpu: [{}] });
    const { done, states } = run({ backends: ["webgpu"], pinned: true, start: fake.start });
    await settle();
    fake.sessions[0].end("lost");
    await done;
    expect(fake.calls).toHaveLength(1);
    expect(states.at(-1)).toMatchObject({ phase: "failed" });
  });

  test("restarts the same backend after a pause before switching, when the policy allows", async () => {
    const fake = fakeStarts({ webgpu: [{}, {}, {}], webgl: [{}] });
    const { slept } = run({
      backends: ["webgpu", "webgl"],
      start: fake.start,
      policy: { restarts: 2, backoffMs: [1000, 5000], stableMs: 60_000 },
    });
    for (let i = 0; i < 3; i++) {
      await settle();
      fake.sessions[i].end("lost");
    }
    await settle();
    expect(fake.calls.map((c) => c.backend)).toEqual(["webgpu", "webgpu", "webgpu", "webgl"]);
    expect(fake.calls[1].notice).toBeNull();
    expect(slept).toEqual([1000, 5000]);
  });

  test("forgets earlier restarts once a session has run for a stable while", async () => {
    const fake = fakeStarts({ webgpu: [{}, {}, {}], webgl: [{}] });
    const { advance } = run({
      backends: ["webgpu", "webgl"],
      start: fake.start,
      policy: { restarts: 1, backoffMs: [1000], stableMs: 60_000 },
    });
    await settle();
    fake.sessions[0].end("lost");
    await settle();
    advance(120_000);
    fake.sessions[1].end("lost");
    await settle();
    expect(fake.calls.map((c) => c.backend)).toEqual(["webgpu", "webgpu", "webgpu"]);
  });
});
