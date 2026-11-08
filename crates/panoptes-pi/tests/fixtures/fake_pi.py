#!/usr/bin/env python3
"""A stand-in for `pi --mode rpc` that speaks just enough of the protocol
for the adapter's tests. Prompts select behaviour:

  "ask"    asks a confirm question and replies with the answer
  "crash"  exits with code 7 straight away
  "tool"   runs a `bash` call that fails with exit code 3, then replies
  "queue"  lists two steering prompts, then one, then none, before replying
  "gate"   starts a `bash` call, asks Yes/No before running it (as a
           permission extension would, tagged the way Panoptes' own
           `--extension` tags it, if one was passed) and replies with the
           answer
  "tool-ask"  starts a call whose tool asks its own Yes/No question, untagged,
           and replies with the answer
  "tools"  replies with the tools it was started with ("all" by default)
  other    replies with "echo: <prompt>" plus a U+2028 inside the text,
           and the MIME types of any images sent with it
"""

import json
import sys

# Panoptes passes its approvals extension; only a real file counts.
extension = sys.argv[sys.argv.index("--extension") + 1] if "--extension" in sys.argv else None
tools = sys.argv[sys.argv.index("--tools") + 1] if "--tools" in sys.argv else "all"
tags_approvals = extension is not None and "panoptes.approval" in open(extension).read()

entries = []
counter = 0
clock = 0
models = [
    {"id": "small", "name": "Small", "provider": "fake", "contextWindow": 32000, "input": ["text"]},
    {"id": "vendor/large", "name": "Large", "provider": "fake", "contextWindow": 200000, "input": ["text", "image"]},
]
model = models[0]
thinking_level = "medium"


def send(record):
    # ensure_ascii=False writes U+2028 raw, to prove the reader splits
    # records only on LF.
    sys.stdout.write(json.dumps(record, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def respond(command, data=None, success=True, error=None):
    record = {"type": "response", "command": command["type"], "success": success}
    if "id" in command:
        record["id"] = command["id"]
    if data is not None:
        record["data"] = data
    if error:
        record["error"] = error
    send(record)


def append(message):
    global counter
    counter += 1
    entry = {
        "type": "message",
        "id": f"e{counter}",
        "parentId": entries[-1]["id"] if entries else None,
        "message": message,
    }
    entries.append(entry)


def tool_run():
    call = {"type": "toolCall", "id": "call-1", "name": "bash", "arguments": {"command": "make"}}
    assistant = {"role": "assistant", "content": [call], "stopReason": "toolUse", "timestamp": tick()}
    send({"type": "message_start", "message": {"role": "assistant", "content": [], "stopReason": "pending"}})
    send({"type": "message_update", "assistantMessageEvent": {"type": "toolcall_start", "contentIndex": 0, "id": "call-1", "toolName": "bash"}})
    send({"type": "message_end", "message": assistant})
    append(assistant)
    send({"type": "tool_execution_start", "toolCallId": "call-1", "toolName": "bash", "args": call["arguments"]})
    send({"type": "tool_execution_update", "toolCallId": "call-1", "toolName": "bash", "args": call["arguments"],
          "partialResult": {"content": [{"type": "text", "text": "building"}]}})
    content = [{"type": "text", "text": "building\nbroken\n\nCommand exited with code 3"}]
    send({"type": "tool_execution_end", "toolCallId": "call-1", "toolName": "bash",
          "result": {"content": content}, "isError": True})
    result = {"role": "toolResult", "toolCallId": "call-1", "toolName": "bash", "content": content,
              "isError": True, "timestamp": tick()}
    send({"type": "message_start", "message": result})
    send({"type": "message_end", "message": result})
    append(result)


def tick():
    # Pi stamps every message; history is matched to live messages by it.
    global clock
    clock += 1
    return clock


def run(text, images):
    content = text
    if images:
        content = [{"type": "text", "text": text}] + [
            {"type": "image", "data": image["data"], "mimeType": image["mimeType"]} for image in images
        ]
    user = {"role": "user", "content": content, "timestamp": tick()}
    send({"type": "agent_start"})
    send({"type": "message_start", "message": user})
    send({"type": "message_end", "message": user})
    append(user)
    reply = f"echo: {text} done"
    if images:
        reply += f" [{len(images)} {images[0]['mimeType']}]"
    if text == "tools":
        reply = f"tools={tools}"
    if text == "tool":
        tool_run()
        reply = "ran it"
    if text == "queue":
        send({"type": "queue_update", "steering": ["a", "b"], "followUp": []})
        send({"type": "queue_update", "steering": ["b"], "followUp": ["c"]})
        send({"type": "queue_update", "steering": [], "followUp": []})
    if text == "gate":
        send({"type": "tool_execution_start", "toolCallId": "call-g", "toolName": "bash", "args": {"command": "rm -rf build"}})
        if tags_approvals:
            send({"type": "extension_ui_request", "id": "s1", "method": "setStatus",
                  "statusKey": "panoptes.approval", "statusText": "call-g"})
        send({"type": "extension_ui_request", "id": "g1", "method": "select",
              "title": "Dangerous command:\n\n  rm -rf build\n\nAllow?", "options": ["Yes", "No"]})
        answer = json.loads(sys.stdin.readline())
        reply = f"gate={answer.get('value')}"
    if text == "tool-ask":
        send({"type": "tool_execution_start", "toolCallId": "call-t", "toolName": "ask_preference", "args": {"question": "Tabs?"}})
        send({"type": "extension_ui_request", "id": "t1", "method": "select", "title": "Tabs?", "options": ["Yes", "No"]})
        answer = json.loads(sys.stdin.readline())
        reply = f"tabs={answer.get('value')}"
    if text == "ask":
        send({"type": "extension_ui_request", "id": "q1", "method": "confirm", "title": "Proceed?"})
        answer = json.loads(sys.stdin.readline())
        reply = f"confirmed={answer.get('confirmed')}"
    assistant = {"role": "assistant", "content": [{"type": "text", "text": reply}], "stopReason": "stop", "timestamp": tick()}
    send({"type": "message_start", "message": {"role": "assistant", "content": [], "stopReason": "pending"}})
    send({"type": "message_update", "assistantMessageEvent": {"type": "text_start", "contentIndex": 0}})
    send({"type": "message_update", "assistantMessageEvent": {"type": "text_delta", "contentIndex": 0, "delta": reply}})
    send({"type": "message_end", "message": assistant})
    append(assistant)
    send({"type": "agent_end", "messages": [], "willRetry": False})
    send({"type": "agent_settled", "aborted": False})


for line in sys.stdin:
    command = json.loads(line)
    kind = command["type"]
    if kind == "get_state":
        respond(command, {
            "sessionId": "fake-session",
            "sessionFile": "/tmp/fake-session.jsonl",
            "model": model,
            "thinkingLevel": thinking_level,
        })
    elif kind == "get_available_models":
        respond(command, {"models": models})
    elif kind == "set_model":
        found = [m for m in models if (m["provider"], m["id"]) == (command["provider"], command["modelId"])]
        if not found:
            respond(command, success=False, error="Model not found")
            continue
        model = found[0]
        respond(command, model)
    elif kind == "get_available_thinking_levels":
        respond(command, {"levels": ["off", "low", "medium", "high"]})
    elif kind == "set_thinking_level":
        thinking_level = command["level"]
        respond(command)
    elif kind == "get_session_stats":
        respond(command, {"contextUsage": {"tokens": 1200, "contextWindow": model["contextWindow"], "percent": 4}})
    elif kind == "prompt":
        if command["message"] == "crash":
            sys.stderr.write("fake pi crashed\n")
            sys.exit(7)
        respond(command, {"disposition": "started"})
        run(command["message"], command.get("images", []))
    elif kind == "get_entries":
        since = command.get("since")
        if since is None:
            found = entries
        else:
            ids = [entry["id"] for entry in entries]
            if since not in ids:
                respond(command, success=False, error=f"Unknown entry {since}")
                continue
            found = entries[ids.index(since) + 1 :]
        respond(command, {"entries": found, "leafId": entries[-1]["id"] if entries else None})
    elif kind == "abort":
        respond(command)
    else:
        respond(command, success=False, error=f"Unknown command {kind}")
