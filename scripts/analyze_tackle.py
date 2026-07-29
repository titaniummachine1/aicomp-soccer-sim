import json, re, sys

path = r'C:\Users\Terminatort8000\AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\Timeplots\timeplot_2026-07-29_01-51-57.json'
with open(path, 'r') as f:
    raw = f.read()

fixed = re.sub(r'(\d),(\d)', r'\1.\2', raw)
data = json.loads(fixed)

names = [s['name'] for s in data['series']]
print("Series names:")
for n in names:
    print(f"  {n}")
print(f"\nTotal series: {len(names)}")
st = data.get("simTime", "?")
print(f"simTime: {st}")

# Build a dict for easy access
series = {s['name']: s for s in data['series']}

# Find possession transitions
def get_vals(name):
    s = series.get(name)
    if not s:
        return [], []
    return s['x'], s['y']

# Look for tackle events: possession changes
# Check Poss.P1HasBall, Poss.P2HasBall transitions
p1_x, p1_y = get_vals("Poss.P1HasBall")
p2_x, p2_y = get_vals("Poss.P2HasBall")

print("\n=== Possession transitions ===")
prev_p1 = 0
prev_p2 = 0
transitions = []
for i in range(len(p1_y)):
    p1 = p1_y[i] if i < len(p1_y) else 0
    p2 = p2_y[i] if i < len(p2_y) else 0
    t = p1_x[i] if i < len(p1_x) else 0
    
    if p1 > 0.5 and prev_p1 <= 0.5:
        print(f"  t={t:.3f}s: P1 GAINS ball")
        transitions.append((t, "P1_GAINS"))
    if p1 <= 0.5 and prev_p1 > 0.5:
        print(f"  t={t:.3f}s: P1 LOSES ball")
        transitions.append((t, "P1_LOSES"))
    if p2 > 0.5 and prev_p2 <= 0.5:
        print(f"  t={t:.3f}s: P2 GAINS ball")
        transitions.append((t, "P2_GAINS"))
    if p2 <= 0.5 and prev_p2 > 0.5:
        print(f"  t={t:.3f}s: P2 LOSES ball")
        transitions.append((t, "P2_LOSES"))
    
    prev_p1 = p1
    prev_p2 = p2

# For each transition, show the distance at that moment
print("\n=== Distances at transition moments ===")
dist_p1p2_x, dist_p1p2_y = get_vals("Dist.P1toP2")
dist_p1b_x, dist_p1b_y = get_vals("Dist.P1toBall")
dist_p2b_x, dist_p2b_y = get_vals("Dist.P2toBall")
dist_p1p2_mb_x, dist_p1p2_mb_y = get_vals("Dist.P1toP2.MinusBodyR")
dist_p1b_mb_x, dist_p1b_mb_y = get_vals("Dist.P1toBall.MinusBodyR")
dist_p2b_mb_x, dist_p2b_mb_y = get_vals("Dist.P2toBall.MinusBodyR")
t1_int_x, t1_int_y = get_vals("T1.Interact")
t2_int_x, t2_int_y = get_vals("T2.Interact")

def val_at(name, t_target):
    xs, ys = get_vals(name)
    if not xs:
        return None
    # Find closest index
    best_i = 0
    best_dt = abs(xs[0] - t_target)
    for i in range(1, len(xs)):
        dt = abs(xs[i] - t_target)
        if dt < best_dt:
            best_dt = dt
            best_i = i
    return ys[best_i]

for t, event in transitions:
    d12 = val_at("Dist.P1toP2", t)
    d1b = val_at("Dist.P1toBall", t)
    d2b = val_at("Dist.P2toBall", t)
    d12mb = val_at("Dist.P1toP2.MinusBodyR", t)
    d1bmb = val_at("Dist.P1toBall.MinusBodyR", t)
    d2bmb = val_at("Dist.P2toBall.MinusBodyR", t)
    i1 = val_at("T1.Interact", t)
    i2 = val_at("T2.Interact", t)
    print(f"\n  {event} at t={t:.3f}s:")
    print(f"    Dist.P1toP2         = {d12}")
    print(f"    Dist.P1toP2.MinusB  = {d12mb}")
    print(f"    Dist.P1toBall       = {d1b}")
    print(f"    Dist.P1toBall.MinusB= {d1bmb}")
    print(f"    Dist.P2toBall       = {d2b}")
    print(f"    Dist.P2toBall.MinusB= {d2bmb}")
    print(f"    T1.Interact         = {i1}")
    print(f"    T2.Interact         = {i2}")

# Also show interact events (when interact pressed)
print("\n=== Interact press events ===")
prev_i1 = 0
prev_i2 = 0
for i in range(len(t1_int_y)):
    i1 = t1_int_y[i] if i < len(t1_int_y) else 0
    i2 = t2_int_y[i] if i < len(t2_int_y) else 0
    t = t1_int_x[i] if i < len(t1_int_x) else 0
    
    if i1 > 0.5 and prev_i1 <= 0.5:
        d12 = val_at("Dist.P1toP2", t)
        d1b = val_at("Dist.P1toBall", t)
        d12mb = val_at("Dist.P1toP2.MinusBodyR", t)
        d1bmb = val_at("Dist.P1toBall.MinusBodyR", t)
        print(f"  t={t:.3f}s: P1 presses E | P1toP2={d12:.4f} P1toP2-MB={d12mb:.4f} P1toBall={d1b:.4f} P1toBall-MB={d1bmb:.4f}")
    if i2 > 0.5 and prev_i2 <= 0.5:
        d12 = val_at("Dist.P1toP2", t)
        d2b = val_at("Dist.P2toBall", t)
        d12mb = val_at("Dist.P1toP2.MinusBodyR", t)
        d2bmb = val_at("Dist.P2toBall.MinusBodyR", t)
        print(f"  t={t:.3f}s: P2 presses RCtrl | P1toP2={d12:.4f} P1toP2-MB={d12mb:.4f} P2toBall={d2b:.4f} P2toBall-MB={d2bmb:.4f}")
    
    prev_i1 = i1
    prev_i2 = i2

# Show min distances while interact pressed
print("\n=== Min distances while interact is pressed ===")
min_d12_p1 = 999
min_d1b_p1 = 999
min_t_p1 = 0
for i in range(len(t1_int_y)):
    i1 = t1_int_y[i] if i < len(t1_int_y) else 0
    t = t1_int_x[i] if i < len(t1_int_x) else 0
    if i1 > 0.5:
        d12 = val_at("Dist.P1toP2", t)
        d1b = val_at("Dist.P1toBall", t)
        if d12 is not None and d12 < min_d12_p1:
            min_d12_p1 = d12
            min_d1b_p1 = d1b if d1b else 999
            min_t_p1 = t

if min_d12_p1 < 999:
    print(f"  P1 interact: min P1toP2={min_d12_p1:.4f} at t={min_t_p1:.3f}s (MinusBodyR={min_d12_p1 - 0.655*2:.4f})")
    print(f"               P1toBall at that moment={min_d1b_p1:.4f} (MinusBodyR={min_d1b_p1 - 0.655:.4f})")

min_d12_p2 = 999
min_d2b_p2 = 999
min_t_p2 = 0
for i in range(len(t2_int_y)):
    i2 = t2_int_y[i] if i < len(t2_int_y) else 0
    t = t2_int_x[i] if i < len(t2_int_x) else 0
    if i2 > 0.5:
        d12 = val_at("Dist.P1toP2", t)
        d2b = val_at("Dist.P2toBall", t)
        if d12 is not None and d12 < min_d12_p2:
            min_d12_p2 = d12
            min_d2b_p2 = d2b if d2b else 999
            min_t_p2 = t

if min_d12_p2 < 999:
    print(f"  P2 interact: min P1toP2={min_d12_p2:.4f} at t={min_t_p2:.3f}s (MinusBodyR={min_d12_p2 - 0.655*2:.4f})")
    print(f"               P2toBall at that moment={min_d2b_p2:.4f} (MinusBodyR={min_d2b_p2 - 0.655:.4f})")
