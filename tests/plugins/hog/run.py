import json, sys, time
sys.stdin.readline()
print(json.dumps({"type": "item", "id": "main", "text": "hog"}), flush=True)
chunks = []
while True:
    chunks.append(bytearray(16 * 1024 * 1024))
    time.sleep(0.05)
