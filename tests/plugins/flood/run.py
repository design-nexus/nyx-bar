import json, sys
sys.stdin.readline()
out = sys.stdout
out.write(json.dumps({"type": "item", "id": "main", "view": {"node": "label", "id": "n", "text": "0"}}) + "\n")
i = 0
while True:
    i += 1
    out.write('{"type":"patch","item":"main","nodes":[{"id":"n","set":{"text":"%d"}}]}\n' % i)
    if i % 100 == 0:
        out.flush()
