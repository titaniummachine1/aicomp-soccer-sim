# Soccer AIComp API dump (no connections)

Pasteable reference for replenishing missing Soccer nodes/labels.

## Getter labels

### SoccerGetBool (19)

- `Ball On Team Side`
- `Is Active Graph`
- `Is Ball Loose`
- `Is Ball Nearby Team Player 1`
- `Is Ball Nearby Team Player 2`
- `Is Ball Nearby Team Player 3`
- `Is Ball Nearby Team Player 4`
- `Is Home Team`
- `Is Team Kicking off`
- `Is Team Player 1 Closest Teammate to Ball`
- `Is Team Player 2 Closest Teammate to Ball`
- `Is Team Player 3 Closest Teammate to Ball`
- `Is Team Player 4 Closest Teammate to Ball`
- `Opponent Has Ball`
- `Team Has Ball`
- `Team Player 1 Has Ball`
- `Team Player 2 Has Ball`
- `Team Player 3 Has Ball`
- `Team Player 4 Has Ball`

### SoccerGetFloat (14)

- `Ball Carrier Shot Charge`
- `Ball Carrier Stamina`
- `Distance from Team Player 1 to nearest Opponent`
- `Distance from Team Player 2 to nearest Opponent`
- `Distance from Team Player 3 to nearest Opponent`
- `Distance from Team Player 4 to nearest Opponent`
- `Opponent Score`
- `Player Interact Radius`
- `Stamina of last defending opponent`
- `Team Player 1 Stamina`
- `Team Player 2 Stamina`
- `Team Player 3 Stamina`
- `Team Player 4 Stamina`
- `Team Score`

### SoccerGetTransform (13)

- `Ball`
- `Opponent Goal Center`
- `Team Goal Center`
- `Team Goal Left Post`
- `Team Goal Right Post`
- `Team Player 1`
- `Team Player 2`
- `Team Player 3`
- `Team Player 4`
- `Teammate Nearest Team Player 1`
- `Teammate Nearest Team Player 2`
- `Teammate Nearest Team Player 3`
- `Teammate Nearest Team Player 4`

### SoccerGetVector3 (51)

- `Backwards clear direction from team carrier`
- `Ball Velocity`
- `Center Field`
- `Clear direction from Teammate 1`
- `Clear direction from Teammate 2`
- `Clear direction from Teammate 3`
- `Clear direction from Teammate 4`
- `Clear direction from team carrier`
- `Clear direction from team carrier (avoid all walls)`
- `Clear direction from team carrier (avoid goal lines)`
- `Clear direction from team carrier (avoid sidelines)`
- `Direction of ball from Teammate 1`
- `Direction of ball from Teammate 2`
- `Direction of ball from Teammate 3`
- `Direction of ball from Teammate 4`
- `Direction of clear teammate from Opponent 1`
- `Direction of clear teammate from Opponent 2`
- `Direction of clear teammate from Opponent 3`
- `Direction of clear teammate from Opponent 4`
- `Direction of clear teammate from Teammate 1`
- `Direction of clear teammate from Teammate 2`
- `Direction of clear teammate from Teammate 3`
- `Direction of clear teammate from Teammate 4`
- `Direction of opponent goal from Teammate 1`
- `Direction of opponent goal from Teammate 2`
- `Direction of opponent goal from Teammate 3`
- `Direction of opponent goal from Teammate 4`
- `Direction of team goal from Teammate 1`
- `Direction of team goal from Teammate 2`
- `Direction of team goal from Teammate 3`
- `Direction of team goal from Teammate 4`
- `Direction of teammate from Team Player 1`
- `Direction of teammate from Team Player 2`
- `Direction of teammate from Team Player 3`
- `Direction of teammate from Team Player 4`
- `Get furthest open opponent`
- `Get furthest open teammate`
- `Get most open opponent`
- `Get most open teammate`
- `Get nearest open opponent`
- `Get nearest open teammate`
- `Lower Corner Away Side`
- `Lower Corner Home Side`
- `Lower Corner Opposing Side`
- `Lower Corner Team Side`
- `Lower Midfield`
- `Upper Corner Away Side`
- `Upper Corner Home Side`
- `Upper Corner Opposing Side`
- `Upper Corner Team Side`
- `Upper Midfield`

## Physics (ball)

- Scale `0.9`, collider radius local `0.45`, mass `0.45`
- PhysicMaterial `Soccer`: bounciness `0.4`, friction `0.1`
- Velocity API: `SoccerGetVector3('Ball Velocity')`

## Field bounds (approx)

- x ∈ [-39.5, 39.5], z ∈ [-24.7, 24.7]

## Sim rules

- Tackles: Subtract stamina delta; tackling player wins ball if they have more stamina AFTER tackle
- Free ball: must interact/tackle to pick up (not automatic)
- Shot: hold interact to charge, set false to release; global pickup lockout ~0.3s after any shot (AIA Jul 18; may be fixed later)
- RelativePosition Self = local frame of input transform, not the controlling player (prefer World diffs)
- Clear dirs home order: E, C, H, B, G, A, F, D
- Clear dirs away order: D, F, A, G, B, H, C, E

Full machine-readable dump: `soccer_api_dump.json`
