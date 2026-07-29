import sys, re, json
path = sys.argv[1]
with open(path, 'r', encoding='utf-8') as f:
    text = f.read()
text = re.sub(r'(?<=[\d\-]),(?=\d)', '.', text)
data = json.loads(text)
print("simTime:", data.get('simTime'))
print("series count:", len(data.get('series', [])))
for s in data['series']:
    name = s['name']
    x = s.get('x', [])
    y = s.get('y', [])
    print("  %s: %d x points, %d y points" % (name, len(x), len(y)))
    if len(y) > 0 and name in ['Ball.X', 'Ball.Y', 'Ball.Z', 'Ball.Speed', 'T1.X', 'T1.Z', 'T2.X', 'T2.Z']:
        print("    y range: %.3f .. %.3f" % (min(y), max(y)))
