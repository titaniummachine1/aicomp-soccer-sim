@echo off
REM Visible PerfectController prediction regression (all 4 parity cases).
REM Builds baseline+compact graphs, runs headless gate, then opens Bevy viewer.
REM
REM In viewer:
REM   Purple path + disc = graph prediction
REM   White ball       = sim rollout from same injected state
REM   1-4              = switch case (center_roll, diag_45, wall_approach, low_speed)
REM   B / C            = baseline / compact graph
REM   R                = restart current case
REM   Debug (top-right) toggles overlay if lines missing
cd /d "%~dp0.."
if "%PERFECT_REGRESS_SKIP_BUILD%"=="" (
  echo Building graphs + running headless parity gate...
  py -3.12 scripts\perfect_prediction_regress.py --build
  if errorlevel 1 (
    echo Regression FAILED — fix before trusting the viewer.
    exit /b 1
  )
) else (
  echo Skipping rebuild ^(gate already passed^).
)
echo.
echo Launching visible regression viewer...
cargo run -- --fast --perfect-regress --away idle --opening home
