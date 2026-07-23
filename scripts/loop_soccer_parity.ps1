# Self-paced Soccer AIA parity loop (fixed 4m heartbeat).
# Stop: kill this process / tell agent to stop the loop.
# Practical parity only — Unity held-ball can still hit board walls (buggy), so
# tiny unpredictable move/rotation residuals are expected; do not chase perfection.
$ErrorActionPreference = 'Continue'
while ($true) {
    Start-Sleep -Seconds 240
    Write-Output 'AGENT_LOOP_TICK_soccerparity {"prompt":"Continue Soccer AIA_Debug vs Unity TimePlot practical parity (DB35 Away open: timeplot_2026-07-23_02-01-06.json). Re-run timeplot_until_goal 2s home=AIA_Debug away=aia opening=away, first2s_rmse.py. Fix next highest-impact driver (O1.Z/Ball.Z walk, post-claim Ball.X, MoveTo tick). Do not expect perfect parity — Unity held-ball vs walls is buggy/noisy. Stop only when first-2s Ball/T1/O1/MoveTo are practically close. Keep FIXED_DT; kickoff_delay=4.9 on parity runs. One independent variable; identical initial snapshot."}'
}
