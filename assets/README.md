# Extracted from AIComp (`Aialanders_Data`)

Source: `sharedassets0.assets` via UnityPy (`scripts/extract_pitch_textures.py`).

| File | Unity name | Use |
|------|------------|-----|
| `grass.png` | `grass_2` (4096²) | Pitch fill (rotated so stripes run goal-to-goal; tiled) |
| `goal_net.png` | `net` (1001²) | Goal netting (black→alpha, white cord) |
| `from_game/*` | raw dumps | Originals + line sprites |

**Pitch chalk lines:** AIComp draws them as geometry (`Default-Line` / line renderers), not a chalk texture — viewer draws scaled rectangles/circles to match Field Width/Depth, Area Depth, kickoff circle.
