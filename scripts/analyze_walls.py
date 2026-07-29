import json, re, sys

path = r'C:\Users\Terminatort8000\AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\Timeplots\timeplot_2026-07-29_02-36-21.json'
with open(path, 'r') as f:
    raw = f.read()

fixed = re.sub(r'(\d),(\d)', r'\1.\2', raw)
data = json.loads(fixed)

series = {s['name']: s['y'] for s in data['series']}

print('=== Dimensions ===')
print('BodyRadius:', series['Dim.BodyRadius'][0])
print('InteractR:', series['Dim.InteractR'][0])
print('SumBodyPlusInteract:', series['Dim.SumBodyPlusInteract'][0])
print('FieldWidth:', series['Dim.FieldWidth'][0])
print('FieldDepth:', series['Dim.FieldDepth'][0])
print('GoalWidth:', series['Dim.GoalWidth'][0])
print('KickoffR:', series['Dim.KickoffR'][0])
print('FixedDT:', series['Dim.FixedDT'][0])
print()
print('=== Corners ===')
print('HomeUp X,Z:', series['Corner.HomeUp.X'][0], series['Corner.HomeUp.Z'][0])
print('HomeLo X,Z:', series['Corner.HomeLo.X'][0], series['Corner.HomeLo.Z'][0])
print('AwayUp X,Z:', series['Corner.AwayUp.X'][0], series['Corner.AwayUp.Z'][0])
print('AwayLo X,Z:', series['Corner.AwayLo.X'][0], series['Corner.AwayLo.Z'][0])
print()
print('=== Center Field ===')
print('Center X,Z:', series['Geom.CenterField.X'][0], series['Geom.CenterField.Z'][0])
print()

t1x = series['T1.X']
t1z = series['T1.Z']
ball_x = series['Ball.X']
ball_z = series['Ball.Z']

print('=== T1 Position (first 20 ticks) ===')
for i in range(20):
    print(f'tick {i}: T1.X={t1x[i]:.4f} T1.Z={t1z[i]:.4f} Ball.X={ball_x[i]:.4f} Ball.Z={ball_z[i]:.4f}')
print()

print('=== T1 Position (ticks 18-30, when stopping at wall) ===')
for i in range(18, 31):
    print(f'tick {i}: T1.X={t1x[i]:.6f} T1.Z={t1z[i]:.6f}')
print()

print('=== T1 min/max X and Z ===')
print(f'T1.X range: {min(t1x):.6f} to {max(t1x):.6f}')
print(f'T1.Z range: {min(t1z):.6f} to {max(t1z):.6f}')
print()

# Wall positions from corners
home_x = series['Corner.HomeUp.X'][0]  # left wall (min X)
away_x = series['Corner.AwayUp.X'][0]  # right wall (max X)
lo_z = series['Corner.HomeLo.Z'][0]    # bottom wall (min Z)
up_z = series['Corner.HomeUp.Z'][0]    # top wall (max Z)

body_r = series['Dim.BodyRadius'][0]
interact_r = series['Dim.InteractR'][0]

print('=== Wall distances ===')
print(f'Left wall (Home X) = {home_x}')
print(f'Right wall (Away X) = {away_x}')
print(f'Bottom wall (Lo Z) = {lo_z}')
print(f'Top wall (Up Z) = {up_z}')
print()
print(f'T1 min X = {min(t1x):.6f}, distance to left wall = {abs(min(t1x) - home_x):.6f}')
print(f'T1 max X = {max(t1x):.6f}, distance to right wall = {abs(away_x - max(t1x)):.6f}')
print(f'T1 min Z = {min(t1z):.6f}, distance to bottom wall = {abs(min(t1z) - lo_z):.6f}')
print(f'T1 max Z = {max(t1z):.6f}, distance to top wall = {abs(up_z - max(t1z)):.6f}')
print()
print(f'Body radius = {body_r}')
print(f'Expected wall clamp = body_r from wall = {body_r}')
print(f'Min X - body_r = {min(t1x) - body_r:.6f} (should be ~{home_x})')
print(f'Min Z - body_r = {min(t1z) - body_r:.6f} (should be ~{lo_z})')
print()

# Check ball position relative to T1 when carrying
print('=== Ball vs T1 when carrying (ticks 30-40) ===')
for i in range(30, 41):
    dx = ball_x[i] - t1x[i]
    dz = ball_z[i] - t1z[i]
    dist = (dx**2 + dz**2)**0.5
    print(f'tick {i}: T1=({t1x[i]:.4f},{t1z[i]:.4f}) Ball=({ball_x[i]:.4f},{ball_z[i]:.4f}) offset=({dx:.4f},{dz:.4f}) dist={dist:.4f}')
