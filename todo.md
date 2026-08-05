# I-69 Deployment Readiness TODO

Use this as the source of truth for the advisor update and deployment decision.
Only check an item after saving the requested evidence.

## Already Verified

- [x] Rust test suite passes (`cargo test`: 17 tests passed).
- [x] Rust formatting check passes (`cargo fmt --all -- --check`).
- [x] Webapp production build succeeds (`pnpm run build`).
- [x] Project documentation site builds (`./scripts/build-docs-site.sh`).
- [x] `i69-mu1` can reach its LabJack at `192.168.1.111` over Ethernet.
- [x] The LabJack serial-number guard rejects a mismatched device configuration.
- [x] The `i69-mu1` NATS leaf connects to central OATS NATS.
- [x] NATS, the NATS metrics exporter, and Alloy start automatically on `i69-mu1`.
- [x] Historical LabJack samples were archived locally as Parquet files.
- [x] Historical Parquet data can be exported as CSV without using the webapp.

## 1. Fix MU1 Configuration and Identity

- [ ] Create a permanent configuration profile for `i69-mu1` instead of reusing the mutable MU2 configuration.
  - Required evidence: a saved MU1 config containing `box_id=i69-mu1`, its correct JetStream domain, KV key, LabJack IP, and LabJack serial `470036312`.
- [ ] Create a separate permanent configuration profile for `i69-mu2`.
  - Required evidence: a saved MU2 config containing `box_id=i69-mu2` and LabJack serial `470036330`.
- [ ] Render the generated NATS and Rust configuration independently on each MU.
  - Required evidence: saved output of the rendered `BOX_ID`, `JS_DOMAIN`, `CFG_KEY`, `LABJACK_IP`, and `LABJACK_SERIAL` on each MU.
- [ ] Change the operating-system hostname on each LattePanda from a generic name such as `localhost.localdomain` to its assigned MU name.
  - Required evidence: `hostname` and `tailscale status --self` identify the same box.
- [ ] Replace the old MU1 `v1` KV key and live subjects with the current namespace.
  - Required evidence: MU1 uses `i69.i69-mu1.i69-lj2.config` and publishes `avenars.i69.i69-mu1.i69-lj2.live.*`.
- [ ] Remove or archive obsolete `labjackd.config.*` and `v1.*` KV entries after migration.
  - Required evidence: central and local KV key listings contain only intentionally supported keys.
- [ ] Verify that every config has a unique and correct combination of site, box, source ID, asset number, LabJack name, and serial number.
  - Required evidence: a two-row MU1/MU2 identity table reviewed by another team member.

## 2. Make the Data Services Survive Reboots

- [ ] Install persistent systemd units for `streamer`, `archiver`, and `exporter`; do not rely only on transient `systemd-run` units.
  - Required evidence: all three units report `enabled` and `active` after a reboot.
- [ ] Configure the services to wait for network, local NATS, storage, and the LabJack where appropriate.
  - Required evidence: a cold boot starts the services in the correct order without manual commands.
- [ ] Confirm the exporter runs in NATS `worker` mode on each edge box.
  - Required evidence: logs show the exact `avenars.i69.<box>.<source>.export.request` subscription.
- [ ] Add a deployment command or script that installs the correct box profile, binaries, and systemd units reproducibly.
  - Required evidence: a clean or reset test host can be configured by following one documented procedure.
- [ ] Reboot each LattePanda three times and confirm acquisition resumes every time.
  - Required evidence: boot timestamps, service status, and first post-boot sample timestamp for all three trials.

## 3. Prevent and Measure Data Loss

- [ ] Make the archiver gracefully flush and close every active Parquet writer during service shutdown.
  - Required evidence: stopping `avena-archiver` leaves no unreadable current part files.
- [ ] Stop aborting channel-writer tasks before their Parquet files are closed during config changes.
  - Required evidence: add/remove-channel tests produce only readable Parquet files.
- [ ] Write active data to a temporary filename and rename it to `.parquet` only after a successful close.
  - Required evidence: the exporter never treats an unfinished file as a completed Parquet file.
- [ ] Decide how startup handles incomplete files: recover, quarantine, or delete them with an audit log.
  - Required evidence: documented behavior and an automated test using a deliberately truncated file.
- [ ] Report corrupt/skipped files to monitoring instead of only printing an exporter warning.
  - Required evidence: a deliberately corrupt file produces a visible metric or alert.
- [ ] Add an end-to-end test for stream to NATS to Parquet to CSV.
  - Required evidence: the test verifies timestamps, channel, raw value, calibrated value, calibration ID, and row count.
- [ ] Run a controlled power-cut test while acquiring data.
  - Required evidence: record the last sample before power loss, first sample after recovery, recovery duration, and number of missing samples.
- [ ] Repeat the power-cut test at least five times.
  - Required evidence: a results table with no unexplained gaps or unreadable completed files.

## 4. Validate Live Acquisition and Central Export

- [ ] Start `streamer`, `archiver`, and `exporter` on MU1 with the corrected MU1 profile.
  - Required evidence: all three services are active and their logs contain no repeating errors.
- [ ] Verify the streamer connects to LabJack serial `470036312` and completes its read/write self-test.
  - Required evidence: saved streamer log excerpt.
- [ ] Receive at least five live batches for every enabled channel from central NATS.
  - Required evidence: saved `nats sub` output containing the current unversioned subjects.
- [ ] Change the enabled channel list in central KV and verify the local KV mirrors it.
  - Required evidence: matching central/local KV JSON and streamer restart log.
- [ ] Confirm all enabled channels publish at the expected cadence and sample rate.
  - Required evidence: a script or report comparing batch counts, sequence numbers, and timestamps by channel.
- [ ] Record continuously for at least 24 hours on MU1.
  - Required evidence: no unexplained service restarts, sequence gaps, corrupt completed files, or disk errors.
- [ ] Export a known 10-minute range through the central NATS worker path without using the webapp.
  - Required evidence: CSV file, exporter logs, byte count, row count, and missing-channel summary.
- [ ] Repeat the same validation on MU2.
  - Required evidence: equivalent MU2 logs, samples, archive, and CSV export.

## 5. Finish Monitoring and Alerts

- [ ] Fix or disable the Alloy XFS collector that currently logs an error every 15 seconds.
  - Required evidence: Alloy runs for one hour without the repeating XFS error.
- [ ] Confirm Prometheus receives host and NATS metrics labeled with the correct MU name.
  - Required evidence: saved query results for both MU1 and MU2.
- [ ] Export a metric for the last successfully published LabJack sample time.
  - Required evidence: the metric updates while streaming and becomes stale when the sensor is disconnected.
- [ ] Export metrics for streamer, archiver, and exporter service state.
  - Required evidence: each service can be distinguished as running, failed, or stopped.
- [ ] Add an alert for a LabJack that stops producing samples.
  - Required evidence: unplugging the LabJack produces an alert within the agreed timeout.
- [ ] Add alerts for a stopped Rust service, lost central leaf connection, low disk space, and exporter failures.
  - Required evidence: trigger and capture each alert once.
- [ ] Add battery voltage, solar charge/current, and Wattdog state to monitoring.
  - Required evidence: current readings are visible with correct box/device labels.
- [ ] Write the alert response procedure, including who receives the alert and what they do.
  - Required evidence: reviewed runbook with contact and escalation information.

## 6. Finish Wattdog, MPPT, and Battery Testing

- [ ] Enable and verify Bluetooth on both LattePandas.
  - Required evidence: `bluetooth.service` is active after reboot.
- [ ] Confirm Wattdog receives Thornwave advertisements reliably.
  - Required evidence: one hour of logs with current readings and no repeated “no advertisements” warning.
- [ ] Install the second Thornwave/Wattdog sensor on the solar side of each box.
  - Required evidence: both load consumption and solar charge rate are recorded simultaneously.
- [ ] Document the relay wiring and verify Wattdog controls the intended relay.
  - Required evidence: labeled wiring diagram and bench-test results.
- [ ] Verify Wattdog removes load power before the MPPT low-voltage cutoff.
  - Required evidence: measured Wattdog cutoff voltage, MPPT cutoff voltage, timestamps, and relay states.
- [ ] Verify the MPPT powers only the PiKVM as intended after the Wattdog cutoff.
  - Required evidence: measured powered/unpowered outputs during cutoff.
- [ ] Select and document low-voltage cutoff, recovery voltage, hysteresis, and delay values.
  - Required evidence: approved settings recorded in the deployment configuration.
- [ ] Simulate battery discharge through the cutoff threshold.
  - Required evidence: voltage/current plot and event log showing the expected shutdown order.
- [ ] Simulate battery recharge through the recovery threshold.
  - Required evidence: voltage/current plot and event log showing the expected startup order.
- [ ] Remove Wattdog `--dry-run` only after the wiring and thresholds pass the bench test.
  - Required evidence: approved change record and a successful controlled relay operation.
- [ ] Repeat at least five complete discharge/recharge cycles.
  - Required evidence: results table showing consistent behavior and software recovery.

## 7. Characterize the ADC Inputs

- [ ] Write the ADC test setup, equipment list, channel configuration, sample rate, and acceptance limits.
  - Required evidence: reviewed one-page test procedure.
- [ ] Verify each ADC input with a known DC source at zero, mid-scale, and near full-scale.
  - Required evidence: raw data and pass/fail table for every deployed input.
- [ ] Measure DC bias with both inputs shorted to ground.
  - Required evidence: mean, minimum, maximum, and standard deviation plot.
- [ ] Measure DC voltage error in volts and percent of full scale.
  - Required evidence: error table and plot across the tested voltage range.
- [ ] Measure input noise with the inputs shorted.
  - Required evidence: time-series plot, histogram, RMS noise, and peak-to-peak noise.
- [ ] Calculate effective number of bits from the noise measurements.
  - Required evidence: formula, assumptions, input data, and ENOB result.
- [ ] Measure frequency response over the required operating band.
  - Required evidence: gain-versus-frequency plot and identified cutoff frequency.
- [ ] Repeat the critical measurements at the intended field scan rate and gain settings.
  - Required evidence: results match the actual deployment configuration.
- [ ] Store the plotting script and source CSV files in a reproducible results directory.
  - Required evidence: another person can regenerate every plot.

## 8. Validate Vehicle Data and INDOT Handoff

- [ ] Define how a vehicle event maps to camera data and LabJack timestamps.
  - Required evidence: documented time synchronization and naming convention.
- [ ] Decide whether per-car downloads require new software or a documented time-range export procedure.
  - Required evidence: advisor and INDOT approve the chosen workflow.
- [ ] Perform a controlled drive-by test with at least five vehicle passes.
  - Required evidence: event log containing vehicle-pass times and expected sensor response.
- [ ] Retrieve the camera file and LabJack data for every test pass.
  - Required evidence: one folder or manifest per pass containing all associated files.
- [ ] Verify LattePanda, LabJack, PiKVM, camera, and central-server clocks are synchronized.
  - Required evidence: measured maximum clock offset between systems.
- [ ] Produce an INDOT sample data package.
  - Required evidence: camera sample, CSV sample, data dictionary, units, calibration information, timestamps, and file-naming description.
- [ ] Ask INDOT to validate that the data format and contents are usable.
  - Required evidence: written acceptance or a documented correction list.

## 9. Complete Firmware and Remote Programming

- [ ] Obtain Spencer’s current SDI-12 firmware and record its source revision.
  - Required evidence: firmware source and build instructions are stored in a controlled repository.
- [ ] Add the required thermocouple firmware support.
  - Required evidence: bench test with known temperatures and recorded error limits.
- [ ] Document the board programming connector, programmer, voltage, cable orientation, and recovery procedure.
  - Required evidence: illustrated programming instructions.
- [ ] Restore Tailscale connectivity to both PiKVMs and MU2.
  - Required evidence: all four assigned I-69 peers appear online and accept their intended remote connection.
- [ ] Connect the required programmer or remote flashing interface to each deployed board.
  - Required evidence: remote operator can identify the target board before programming.
- [ ] Program one board remotely and verify the new firmware version afterward.
  - Required evidence: session log, firmware hash/version, and successful functional test.
- [ ] Test rollback or recovery from an interrupted firmware update.
  - Required evidence: documented recovery test completed without physical access.

## 10. Security and Remote Access

- [ ] Replace the shared device password with unique credentials or SSH keys for every device.
  - Required evidence: device access inventory showing the authentication method and authorized users.
- [ ] Disable password SSH login after key-based access and recovery access are verified.
  - Required evidence: key login succeeds and password login is rejected.
- [ ] Restrict Tailscale access to the minimum required users and device-to-device paths.
  - Required evidence: reviewed ACL policy and successful allowed/denied access tests.
- [ ] Protect NATS credential files with least-privilege permissions and subjects.
  - Required evidence: file permissions and NATS permission tests for each service identity.
- [ ] Document credential rotation and lost-device response procedures.
  - Required evidence: reviewed security runbook.

## 11. Solar and Installation Dry Run

- [ ] Obtain INDOT approval for the mount design.
  - Required evidence: written signoff and final drawing revision.
- [ ] Inventory all mounting hardware and record quantities.
  - Required evidence: checked bill of materials with labeled storage locations.
- [ ] Inventory outdoor wire, ferrules, crimps, connectors, fuses, Ethernet cables, and weatherproofing supplies.
  - Required evidence: checked consumables list with required spares.
- [ ] Identify every installation tool and confirm the team knows how to use it.
  - Required evidence: tool checklist and assigned operator.
- [ ] Prepare a field-spares kit for likely failures.
  - Required evidence: packed and labeled kit containing cables, wires, ferrules, fuses, connectors, and replacement hardware.
- [ ] Perform a complete mock installation at ACRE or another approved outdoor site.
  - Required evidence: installation photos, elapsed time, issue list, and revised procedure.
- [ ] Verify the solar charge cycle under actual sunlight and expected load.
  - Required evidence: at least one full-day plot of panel, battery, and load voltage/current.
- [ ] Verify weather sealing, cable strain relief, grounding, and enclosure temperature.
  - Required evidence: inspection checklist and temperature measurements.
- [ ] Perform the vehicle-data, monitoring-alert, power-cycle, and remote-access tests during the dry run.
  - Required evidence: one combined dry-run report containing results and failures.
- [ ] Close every issue found during the dry run or document an accepted mitigation.
  - Required evidence: issue list with owner, resolution, verification, and signoff.

## 12. Final Deployment Gate

- [ ] Run the complete deployment procedure on MU1 from a clean boot.
- [ ] Run the complete deployment procedure on MU2 from a clean boot.
- [ ] Confirm both PiKVMs are remotely reachable.
- [ ] Confirm both LabJacks have the correct IP and serial identity.
- [ ] Confirm both boxes stream live data to central NATS.
- [ ] Confirm both boxes archive readable Parquet continuously.
- [ ] Confirm both boxes answer central NATS CSV export requests.
- [ ] Confirm every configured alert reaches the responsible person.
- [ ] Confirm the power system passes the agreed cutoff and recovery tests.
- [ ] Confirm the ADC characterization meets the agreed acceptance limits.
- [ ] Confirm INDOT has accepted the mount and sample data package.
- [ ] Confirm the ACRE dry-run issue list is closed.
- [ ] Back up deployed configs, firmware versions, credentials inventory, and test results.
- [ ] Obtain advisor approval to deploy on I-69.

## Advisor Demonstration Without Repeating the Webapp

- [ ] Show both edge boxes and PiKVMs online in Tailscale.
- [ ] Show the LabJack identity/self-test in the streamer logs.
- [ ] Show live samples arriving through central NATS from the command line.
- [ ] Show Parquet files being created and closed on the edge box.
- [ ] Export a time range to CSV through NATS from the command line.
- [ ] Open the CSV and explain timestamps, raw values, calibrated values, and calibration IDs.
- [ ] Disconnect a LabJack and show the offline alert.
- [ ] Restore the LabJack and show automatic acquisition recovery.
- [ ] Perform a controlled power interruption and show restart plus the measured data gap.
- [ ] Present the ADC plots, power-cycle results, dry-run report, and remaining red/yellow/green deployment items.
