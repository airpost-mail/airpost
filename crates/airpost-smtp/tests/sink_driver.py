"""
Runs the harness SMTP sink (apps/engine/test/fakeimap/smtp.py) for the tern-smtp integration
tests. argv[1] is the harness test directory, argv[2] a JSON object of FakeSmtpServer options.
Prints {"port": N} once listening, then answers each stdin line:
  dump -> {"messages": [{helo, sender, recipients, raw}], "transcript": [[dir, line], ...]}
  quit -> exits
"""
import json
import sys

sys.path.insert(0, sys.argv[1])
from fakeimap.smtp import FakeSmtpServer  # noqa: E402

opts = json.loads(sys.argv[2])
if "credentials" in opts:
    opts["credentials"] = tuple(opts["credentials"])
server = FakeSmtpServer(**opts).start()
print(json.dumps({"port": server.port}), flush=True)

for line in sys.stdin:
    cmd = line.strip()
    if cmd == "dump":
        with server.lock:
            messages = [
                {
                    "helo": m.helo,
                    "sender": m.sender,
                    "recipients": m.recipients,
                    "raw": m.raw.decode("utf-8", "replace"),
                }
                for m in server.messages
            ]
        transcript = [[d, l] for (_, d, l) in list(server.transcript)]
        print(json.dumps({"messages": messages, "transcript": transcript}), flush=True)
    elif cmd == "quit":
        break

server.stop()
