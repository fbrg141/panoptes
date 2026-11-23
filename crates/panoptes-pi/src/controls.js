// Session controls that Pi's RPC protocol does not expose directly. Panoptes
// invokes the command through Pi's normal prompt handling; the status line
// reports the resulting state without adding the command to Panoptes' UI.
const CODEMODE = "codemode";
const STATUS = "panoptes.codemode";
const COMMAND = "panoptes-codemode";

export default function (pi) {
	const publish = (ctx) => {
		const enabled = pi.getActiveTools().includes(CODEMODE);
		ctx.ui.setStatus(STATUS, enabled ? "on" : "off");
	};

	pi.on("session_start", (_event, ctx) => publish(ctx));
	// MCP can activate codemode after startup. Refresh the reported state before
	// a run; Panoptes reads settings again when that run settles.
	pi.on("before_agent_start", (_event, ctx) => publish(ctx));

	pi.registerCommand(COMMAND, {
		description: "Toggle codemode for this session",
		handler: async (args, ctx) => {
			const enabled = args.trim() === "on";
			const active = pi.getActiveTools();
			const next = enabled
				? [...new Set([...active, CODEMODE])]
				: active.filter((name) => name !== CODEMODE);
			pi.setActiveTools(next);
			publish(ctx);
		},
	});
}
