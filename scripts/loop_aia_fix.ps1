# Loop wake for AIA TimePlot parity work
while ($true) {
    Start-Sleep -Seconds 180
    Write-Output 'AGENT_LOOP_TICK_aiafix {"prompt":"Continue AIA vs real TimePlot parity: tackle/interact, MoveTo.Z, possession; run timeplot_until_goal 8s, compare to timeplot_2026-07-22_04-00-07.json, fix next culprit. Stop when first-2s RMSE and Ball.Z span are close."}'
}
