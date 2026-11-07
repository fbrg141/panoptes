// Loaded into every Pi that Panoptes starts. Pi asks for permission from
// extension `tool_call` hooks, but over RPC such a dialog looks exactly like a
// question a tool asks while it runs. This hook runs first (Pi loads `-e`
// extensions before the user's) and gives the hooks after it a `ctx.ui`
// whose confirm and select first send a status line naming the call. Panoptes
// reads a dialog right after that line as the call's approval. Pi writes both
// lines in the same tick, so nothing can come between them.
export default function (pi) {
	pi.on("tool_call", (event, ctx) => {
		const ui = ctx.ui;
		const tag = () => ui.setStatus("panoptes.approval", event.toolCallId);
		const tagged = new Proxy(ui, {
			get(target, key) {
				if (key === "select" || key === "confirm") {
					return (...args) => {
						tag();
						return target[key](...args);
					};
				}
				const value = target[key];
				return typeof value === "function" ? value.bind(target) : value;
			},
		});
		try {
			Object.defineProperty(ctx, "ui", { value: tagged, configurable: true });
		} catch {
			// The hooks see the plain dialogs, which Panoptes shows as questions.
		}
		return undefined;
	});
}
