# TODO

## Part 1: LabJack Detection
- [x] Make production streamer use strict `LABJACK_IP` when set
- [x] Remove production fallback local-scan and USB fallback from streamer open path
- [x] Verify connected device serial `470036330` after connect
- [x] Add minimal LabJack self-test on connect
- [x] Keep direct, explicit failure logging for connect/open/verify errors

## Part 2: Real-Time Plotting
- [x] Move stream payload to numeric timing metadata
- [x] Use source timestamps in the web plot
- [x] Fix auto Y-scale behavior
- [x] Fix trigger capture so pre-trigger data is snapshotted once
- [x] Remove holdoff from trigger UI/logic
- [x] Decouple ingest from render with bounded channel buffers
- [x] Reduce reactive churn in the web plot path for `14 x 5 kHz`
- [ ] Keep live lag bounded under high-rate multi-channel load
- [ ] Evaluate heavier plotting architecture: worker-based parsing, typed ring buffers, and likely WebGL
- [ ] Re-check waveform shape and numeric agreement against LJStreamM

## Part 3: Validation Suite
- [x] Add configurable validation harness
- [x] Load LJStreamM CSV reference data
- [x] Align our Parquet samples by timestamp
- [x] Compute absolute error, relative error, and drift
- [x] Emit summary report and pass/fail result

## Part 4: Deployment
- [ ] Keep Tauri as the preferred deployment direction
- [ ] Define concrete desktop packaging/update path
