// Runs `approvals.js` the way Pi does: Pi calls the default export with its
// extension API, the extension registers a `tool_call` hook, and that hook
// runs before the user's own permission hooks with the same `ctx`. Whether a
// real Pi hands later hooks that same `ctx` is only proven by `pnpm smoke`
// and by use; this proves what the file itself does.

import { describe, expect, it } from "vitest";
import install from "./approvals.js";

/** A stand-in for Pi's extension API that keeps the registered hooks. */
function fakePi() {
  const hooks = [];
  return {
    hooks,
    on(event, handler) {
      hooks.push({ event, handler });
    },
  };
}

/** A `ctx.ui` that records, in order, everything called on it. */
function fakeUi(log) {
  return {
    name: "ui",
    setStatus: (key, text) => log.push(["setStatus", key, text]),
    confirm: async (title) => {
      log.push(["confirm", title]);
      return true;
    },
    select: async (title, options) => {
      log.push(["select", title, options]);
      return options[0];
    },
    input: async (title) => {
      log.push(["input", title]);
      return "typed";
    },
    whoAmI() {
      return this.name;
    },
  };
}

function run() {
  const pi = fakePi();
  install(pi);
  const log = [];
  const ctx = { ui: fakeUi(log) };
  expect(pi.hooks.map((hook) => hook.event)).toEqual(["tool_call"]);
  const result = pi.hooks[0].handler({ toolCallId: "call-7", toolName: "bash" }, ctx);
  return { ctx, log, result };
}

describe("approvals.js", () => {
  it("lets the call through and decides nothing itself", () => {
    const { result, log } = run();
    expect(result).toBeUndefined();
    expect(log).toEqual([]);
  });

  it("names the call in a status line right before a later hook's confirm or select", async () => {
    const { ctx, log } = run();
    expect(await ctx.ui.confirm("Run rm -rf build?")).toBe(true);
    expect(await ctx.ui.select("Allow?", ["Yes", "No"])).toBe("Yes");
    expect(log).toEqual([
      ["setStatus", "panoptes.approval", "call-7"],
      ["confirm", "Run rm -rf build?"],
      ["setStatus", "panoptes.approval", "call-7"],
      ["select", "Allow?", ["Yes", "No"]],
    ]);
  });

  it("leaves every other dialog and method as it was", async () => {
    const { ctx, log } = run();
    expect(await ctx.ui.input("Name?")).toBe("typed");
    expect(ctx.ui.whoAmI()).toBe("ui");
    expect(ctx.ui.name).toBe("ui");
    expect(log).toEqual([["input", "Name?"]]);
  });

  it("does not break the call when `ctx.ui` cannot be replaced", () => {
    const pi = fakePi();
    install(pi);
    const ctx = Object.freeze({ ui: fakeUi([]) });
    expect(() => pi.hooks[0].handler({ toolCallId: "call-8" }, ctx)).not.toThrow();
  });
});
