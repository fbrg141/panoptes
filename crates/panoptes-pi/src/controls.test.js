import assert from "node:assert/strict";
import { describe, it } from "node:test";
import install from "./controls.js";

function fakePi(activeTools) {
	const handlers = new Map();
	const commands = new Map();
	const statuses = [];
	return {
		activeTools,
		handlers,
		commands,
		statuses,
		on(event, handler) {
			handlers.set(event, handler);
		},
		registerCommand(name, definition) {
			commands.set(name, definition);
		},
		getActiveTools() {
			return [...this.activeTools];
		},
		setActiveTools(next) {
			this.activeTools = next;
		},
	};
}

function context(pi) {
	return {
		ui: {
			setStatus: (key, text) => pi.statuses.push([key, text]),
		},
	};
}

describe("controls.js", () => {
	it("reports codemode's actual state when the session starts", () => {
		const pi = fakePi(["read", "codemode"]);
		install(pi);
		pi.handlers.get("session_start")({}, context(pi));
		assert.deepEqual(pi.statuses, [["panoptes.codemode", "on"]]);
	});

	it("enables codemode without dropping another active tool", async () => {
		const pi = fakePi(["read", "bash"]);
		install(pi);
		await pi.commands.get("panoptes-codemode").handler("on", context(pi));
		assert.deepEqual(pi.activeTools, ["read", "bash", "codemode"]);
		assert.deepEqual(pi.statuses, [["panoptes.codemode", "on"]]);
	});

	it("disables only codemode and is idempotent", async () => {
		const pi = fakePi(["read", "codemode"]);
		install(pi);
		const command = pi.commands.get("panoptes-codemode");
		await command.handler("off", context(pi));
		await command.handler("off", context(pi));
		assert.deepEqual(pi.activeTools, ["read"]);
		assert.deepEqual(pi.statuses, [
			["panoptes.codemode", "off"],
			["panoptes.codemode", "off"],
		]);
	});

	it("reports activation that happened between runs", () => {
		const pi = fakePi(["read"]);
		install(pi);
		pi.activeTools.push("codemode");
		pi.handlers.get("before_agent_start")({}, context(pi));
		assert.deepEqual(pi.statuses, [["panoptes.codemode", "on"]]);
	});
});
