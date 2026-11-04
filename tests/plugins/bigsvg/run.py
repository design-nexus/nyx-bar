import json, sys, time
sys.stdin.readline()
svg = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'>" + "<rect width='1' height='1'/>" * 6000 + "</svg>"
print(json.dumps({"type": "item", "id": "main", "view": {"node": "vector", "svg": svg, "width": 16, "height": 16}}), flush=True)
print(json.dumps({"type": "item", "id": "small", "text": "svg ok"}), flush=True)
for line in sys.stdin:
    pass
