#!/usr/bin/env python3
"""A stand-in for `pi --mode rpc` that speaks just enough of the protocol
for the adapter's tests. Prompts select behaviour:

  "ask"    asks a confirm question and replies with the answer
  "crash"  exits with code 7 straight away
  other    replies with "echo: <prompt>" plus a U+2028 inside the text
"""

import json
import sys

entries = []
counter = 0


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


def run(text):
    user = {"role": "user", "content": text, "timestamp": 1}
    send({"type": "agent_start"})
    send({"type": "message_start", "message": user})
    send({"type": "message_end", "message": user})
    append(user)
    reply = f"echo: {text} done"
    if text == "ask":
        send({"type": "extension_ui_request", "id": "q1", "method": "confirm", "title": "Proceed?"})
        answer = json.loads(sys.stdin.readline())
        reply = f"confirmed={answer.get('confirmed')}"
    assistant = {"role": "assistant", "content": [{"type": "text", "text": reply}], "stopReason": "stop", "timestamp": 2}
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
        respond(command, {"sessionId": "fake-session", "sessionFile": "/tmp/fake-session.jsonl"})
    elif kind == "prompt":
        if command["message"] == "crash":
            sys.stderr.write("fake pi crashed\n")
            sys.exit(7)
        respond(command, {"disposition": "started"})
        run(command["message"])
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
