import json, re

path = r'C:\Users\Terminatort8000\AppData\LocalLow\Unicorn One\AIComp\Saves\Soccer\Timeplots\timeplot_2026-07-29_02-36-21.json'
with open(path, 'r') as f:
    raw = f.read()

fixed = re.sub(r'(\d),(\d)', r'\1.\2', raw)
data = json.loads(fixed)

series = {s['name']: s['y'] for s in data['series']}

t1x = series['T1.X']
t1z = series['T1.Z']
ball_x = series['Ball.X']
ball_z = series['Ball.Z']
body_r = series['Dim.BodyRadius'][0]
interact_r = series['Dim.InteractR'][0]

# Find tick where T1 is at min X (left wall)
min_x_tick = t1x.index(min(t1x))
min_z_tick = t1z.index(min(t1z))

print(f'=== T1 at min X (left wall) ===')
print(f'Tick {min_x_tick}: T1.X={t1x[min_x_tick]:.6f} T1.Z={t1z[min_x_tick]:.6f}')
print(f'  Ball.X={ball_x[min_x_tick]:.6f} Ball.Z={ball_z[min_x_tick]:.6f}')
print(f'  Distance to left wall (-40): {abs(t1x[min_x_tick] - (-40)):.6f}')
print(f'  Body radius: {body_r}')
print(f'  Excess: {abs(t1x[min_x_tick] - (-40)) - body_r:.6f}')
print(f'  Ball offset from player: ({ball_x[min_x_tick]-t1x[min_x_tick]:.4f}, {ball_z[min_x_tick]-t1z[min_x_tick]:.4f})')
print(f'  Ball dist from player: {((ball_x[min_x_tick]-t1x[min_x_tick])**2+(ball_z[min_x_tick]-t1z[min_x_tick])**2)**0.5:.4f}')
print()

print(f'=== T1 at min Z (bottom wall) ===')
print(f'Tick {min_z_tick}: T1.X={t1x[min_z_tick]:.6f} T1.Z={t1z[min_z_tick]:.6f}')
print(f'  Ball.X={ball_x[min_z_tick]:.6f} Ball.Z={ball_z[min_z_tick]:.6f}')
print(f'  Distance to bottom wall (-25): {abs(t1z[min_z_tick] - (-25)):.6f}')
print(f'  Body radius: {body_r}')
print(f'  Excess: {abs(t1z[min_z_tick] - (-25)) - body_r:.6f}')
print(f'  Ball offset from player: ({ball_x[min_z_tick]-t1x[min_z_tick]:.4f}, {ball_z[min_z_tick]-t1z[min_z_tick]:.4f})')
print(f'  Ball dist from player: {((ball_x[min_z_tick]-t1x[min_z_tick])**2+(ball_z[min_z_tick]-t1z[min_z_tick])**2)**0.5:.4f}')
print()

# Show ticks around min X to see approach
print(f'=== Approaching left wall (ticks {min_x_tick-5} to {min_x_tick+5}) ===')
for i in range(max(0,min_x_tick-5), min(len(t1x), min_x_tick+6)):
    print(f'  tick {i}: T1.X={t1x[i]:.6f} T1.Z={t1z[i]:.6f} Ball=({ball_x[i]:.4f},{ball_z[i]:.4f})')
print()

# Show ticks around min Z
print(f'=== Approaching bottom wall (ticks {min_z_tick-5} to {min_z_tick+5}) ===')
for i in range(max(0,min_z_tick-5), min(len(t1z), min_z_tick+6)):
    print(f'  tick {i}: T1.X={t1x[i]:.6f} T1.Z={t1z[i]:.6f} Ball=({ball_x[i]:.4f},{ball_z[i]:.4f})')
print()

# Check if player is in goal area when at left wall
goal_width = series['Dim.GoalWidth'][0]  # 11.4
print(f'Goal width: {goal_width}, goal spans Z from {-goal_width/2} to {goal_width/2}')
print(f'T1.Z at left wall: {t1z[min_x_tick]:.4f} (in goal area: {abs(t1z[min_x_tick]) < goal_width/2})')
print()

# Check the ball offset direction when at walls
print('=== Ball offset direction at left wall ===')
bx_off = ball_x[min_x_tick] - t1x[min_x_tick]
bz_off = ball_z[min_x_tick] - t1z[min_x_tick]
print(f'Ball offset: ({bx_off:.4f}, {bz_off:.4f})')
print(f'Ball is {"left" if bx_off < 0 else "right"} of player, {"below" if bz_off < 0 else "above"} of player')
print(f'Ball X relative to wall: {ball_x[min_x_tick]:.4f} (wall at -40)')
print(f'Ball distance to wall: {abs(ball_x[min_x_tick] - (-40)):.6f}')
