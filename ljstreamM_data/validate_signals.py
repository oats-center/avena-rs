#!/usr/bin/env python3

from __future__ import annotations

import argparse
import asyncio
import csv as csv_module
import json
import math
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable

import numpy as np
import pandas as pd
import pyarrow.parquet as pq
import scipy.signal
import websockets


REFERENCE_CHANNEL_COUNT = 14
CSV_REQUIRED_COLUMNS = {
    "timestamp",
    "channel",
    "raw_value",
    "calibrated_value",
    "calibration_id",
}


@dataclass
class SignalSeries:
    name: str
    times_sec: np.ndarray
    values: np.ndarray
    dt_sec: float
    sample_rate_hz: float

    @property
    def samples(self) -> int:
        return int(self.times_sec.size)

    @property
    def duration_sec(self) -> float:
        return float(self.times_sec[-1] - self.times_sec[0])


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Inspect LJStreamM references, fetch CSV exports, and compare signals."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    inspect_parser = subparsers.add_parser(
        "inspect-ref", help="Inspect an LJStreamM .dat reference file."
    )
    inspect_parser.add_argument("--reference", required=True, type=Path)
    inspect_parser.set_defaults(func=cmd_inspect_ref)

    fetch_parser = subparsers.add_parser(
        "fetch-export", help="Fetch a CSV export from the exporter WebSocket endpoint."
    )
    fetch_parser.add_argument("--ws", required=True)
    fetch_parser.add_argument("--asset", required=True, type=int)
    fetch_parser.add_argument("--channels", required=True)
    fetch_parser.add_argument("--start", required=True)
    fetch_parser.add_argument("--end", required=True)
    fetch_parser.add_argument("--out", required=True, type=Path)
    fetch_parser.set_defaults(func=cmd_fetch_export)

    compare_parser = subparsers.add_parser(
        "compare", help="Compare a reference .dat file with CSV or parquet input."
    )
    compare_parser.add_argument("--reference", required=True, type=Path)
    compare_parser.add_argument("--reference-channel", required=True, type=int)
    compare_parser.add_argument("--input", required=True, type=Path)
    compare_parser.add_argument(
        "--input-format", required=True, choices=("csv", "parquet")
    )
    compare_parser.add_argument(
        "--signal-column", required=True, choices=("raw_value", "calibrated_value")
    )
    compare_parser.add_argument("--out-dir", required=True, type=Path)
    compare_parser.add_argument("--plot", action="store_true")
    compare_parser.set_defaults(func=cmd_compare)

    suite_parser = subparsers.add_parser(
        "run-suite",
        help="Run a configurable validation suite and emit pass/fail reports.",
    )
    suite_parser.add_argument("--config", required=True, type=Path)
    suite_parser.add_argument(
        "--out-dir",
        type=Path,
        help="Override the output directory from the suite config.",
    )
    suite_parser.add_argument(
        "--plot",
        action="store_true",
        help="Render plots for every case, in addition to per-case config.",
    )
    suite_parser.set_defaults(func=cmd_run_suite)

    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        result = args.func(args)
        if asyncio.iscoroutine(result):
            result = asyncio.run(result)
    except KeyboardInterrupt:
        print("Interrupted.", file=sys.stderr)
        return 130
    except Exception as exc:  # noqa: BLE001
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return int(result or 0)


def split_fields(line: str) -> list[str]:
    stripped = line.strip()
    if not stripped:
        return []
    if "," in stripped:
        try:
            return next(csv_module.reader([stripped]))
        except csv_module.Error:
            return [field.strip() for field in stripped.split(",")]
    if "\t" in stripped:
        return [field for field in re.split(r"\t+", stripped) if field]
    return re.split(r"\s+", stripped)


def try_parse_float_row(fields: Iterable[str]) -> list[float] | None:
    values: list[float] = []
    for field in fields:
        try:
            values.append(float(field))
        except ValueError:
            return None
    return values


def coerce_signal(name: str, times_sec: np.ndarray, values: np.ndarray) -> SignalSeries:
    times = np.asarray(times_sec, dtype=np.float64)
    vals = np.asarray(values, dtype=np.float64)

    if times.ndim != 1 or vals.ndim != 1:
        raise ValueError(f"{name}: expected 1-D arrays")
    if times.size != vals.size:
        raise ValueError(f"{name}: timestamp/value length mismatch")
    if times.size < 2:
        raise ValueError(f"{name}: need at least two samples")

    finite_mask = np.isfinite(times) & np.isfinite(vals)
    times = times[finite_mask]
    vals = vals[finite_mask]
    if times.size < 2:
        raise ValueError(f"{name}: fewer than two finite samples remain")

    order = np.argsort(times, kind="mergesort")
    times = times[order]
    vals = vals[order]

    unique_times, unique_idx = np.unique(times, return_index=True)
    times = unique_times
    vals = vals[unique_idx]
    if times.size < 2:
        raise ValueError(f"{name}: need at least two unique timestamps")

    diffs = np.diff(times)
    diffs = diffs[diffs > 0]
    if diffs.size == 0:
        raise ValueError(f"{name}: timestamps are not strictly increasing")

    dt_sec = float(np.median(diffs))
    if dt_sec <= 0:
        raise ValueError(f"{name}: invalid median dt {dt_sec}")

    return SignalSeries(
        name=name,
        times_sec=times,
        values=vals,
        dt_sec=dt_sec,
        sample_rate_hz=float(1.0 / dt_sec),
    )


def load_reference_table(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(path)

    header_seen = False
    row_width: int | None = None
    rows: list[list[float]] = []

    with path.open("r", encoding="utf-8-sig", errors="replace") as handle:
        for line in handle:
            fields = split_fields(line)
            if not fields:
                continue
            if not header_seen:
                if fields[0].lower() == "time":
                    header_seen = True
                continue

            parsed = try_parse_float_row(fields)
            if parsed is None:
                continue

            if row_width is None:
                row_width = len(parsed)
                if row_width < 2:
                    raise ValueError(f"{path}: first numeric row is too short")

            if len(parsed) != row_width:
                continue

            rows.append(parsed[:row_width])

    if row_width is None or not rows:
        raise ValueError(f"{path}: no numeric sample rows found")

    matrix = np.asarray(rows, dtype=np.float64)
    available_channels = list(range(min(REFERENCE_CHANNEL_COUNT, row_width - 1)))
    if not available_channels:
        raise ValueError(f"{path}: no vN channels detected")

    times = np.asarray(matrix[:, 0], dtype=np.float64)
    signal_matrix = np.asarray(
        matrix[:, 1 : 1 + len(available_channels)],
        dtype=np.float64,
    )

    finite_mask = np.isfinite(times) & np.all(np.isfinite(signal_matrix), axis=1)
    times = times[finite_mask]
    signal_matrix = signal_matrix[finite_mask]
    if times.size < 2:
        raise ValueError(f"{path}: fewer than two finite reference rows remain")

    order = np.argsort(times, kind="mergesort")
    times = times[order]
    signal_matrix = signal_matrix[order]

    unique_times, unique_idx = np.unique(times, return_index=True)
    times = unique_times
    signal_matrix = signal_matrix[unique_idx]
    if times.size < 2:
        raise ValueError(f"{path}: fewer than two unique reference timestamps remain")

    diffs = np.diff(times)
    diffs = diffs[diffs > 0]
    if diffs.size == 0:
        raise ValueError(f"{path}: reference timestamps are not strictly increasing")

    dt_sec = float(np.median(diffs))
    sample_rate_hz = float(1.0 / dt_sec)

    return {
        "path": path,
        "rows": int(times.size),
        "row_width": row_width,
        "times_sec": times,
        "signals": signal_matrix,
        "dt_sec": dt_sec,
        "sample_rate_hz": sample_rate_hz,
        "available_channels": available_channels,
    }


def load_reference_signal(path: Path, channel: int) -> SignalSeries:
    table = load_reference_table(path)
    if channel not in table["available_channels"]:
        raise ValueError(
            f"{path}: reference channel {channel} unavailable; found {table['available_channels']}"
        )

    channel_index = table["available_channels"].index(channel)
    values = table["signals"][:, channel_index]
    return coerce_signal(
        f"reference {path.name} ch{channel:02d}",
        table["times_sec"],
        values,
    )


def relative_errors(errors: np.ndarray, reference_values: np.ndarray) -> np.ndarray:
    scale = np.maximum(np.abs(reference_values), np.finfo(np.float64).eps)
    return np.abs(errors) / scale


def load_csv_signal(path: Path, channel: int, signal_column: str) -> SignalSeries:
    if not path.exists():
        raise FileNotFoundError(path)

    df = pd.read_csv(path)
    missing = CSV_REQUIRED_COLUMNS.difference(df.columns)
    if missing:
        raise ValueError(f"{path}: missing CSV columns: {sorted(missing)}")

    channel_token = f"ch{channel:02d}"
    df = df[df["channel"].astype(str) == channel_token].copy()
    if df.empty:
        raise ValueError(f"{path}: no rows for channel {channel_token}")

    timestamps = pd.to_datetime(
        df["timestamp"],
        utc=True,
        errors="coerce",
        format="mixed",
    )
    values = pd.to_numeric(df[signal_column], errors="coerce")
    valid = timestamps.notna() & values.notna()
    df = df.loc[valid].copy()
    timestamps = timestamps.loc[valid]
    values = values.loc[valid]
    if df.empty:
        raise ValueError(f"{path}: no valid timestamps/values after filtering")

    seconds = (timestamps - timestamps.min()).dt.total_seconds().to_numpy(dtype=np.float64)
    return coerce_signal(
        f"csv {path.name} ch{channel:02d} {signal_column}",
        seconds,
        values.to_numpy(dtype=np.float64),
    )


def collect_parquet_files(path: Path) -> list[Path]:
    if path.is_file():
        if path.suffix != ".parquet":
            raise ValueError(f"{path}: expected a .parquet file")
        return [path]

    if not path.is_dir():
        raise FileNotFoundError(path)

    files = sorted(path.glob("part-*.parquet"))
    if not files:
        raise ValueError(f"{path}: no part-*.parquet files found")
    return files


def load_parquet_signal(path: Path, signal_column: str) -> SignalSeries:
    if signal_column != "raw_value":
        raise ValueError(
            "archived parquet only stores raw channel values; use --signal-column raw_value"
        )

    timestamp_chunks: list[np.ndarray] = []
    value_chunks: list[np.ndarray] = []
    for file_path in collect_parquet_files(path):
        table = pq.read_table(file_path, columns=["timestamp_unix_ns", "value"])
        ts = table["timestamp_unix_ns"].to_numpy(zero_copy_only=False)
        values = table["value"].to_numpy(zero_copy_only=False)
        if ts.size == 0:
            continue
        timestamp_chunks.append(np.asarray(ts, dtype=np.int64))
        value_chunks.append(np.asarray(values, dtype=np.float64))

    if not timestamp_chunks:
        raise ValueError(f"{path}: parquet input contained no rows")

    timestamps_ns = np.concatenate(timestamp_chunks)
    values = np.concatenate(value_chunks)
    seconds = (
        timestamps_ns - np.min(timestamps_ns)
    ).astype(np.float64) / 1_000_000_000.0
    return coerce_signal(f"parquet {path}", seconds, values)


def normalize_for_xcorr(values: np.ndarray) -> np.ndarray:
    centered = np.asarray(values, dtype=np.float64) - float(np.mean(values))
    rms = float(np.sqrt(np.mean(centered**2)))
    if not np.isfinite(rms) or rms == 0.0:
        raise ValueError("signal RMS is zero after detrending")
    return centered / rms


def estimate_initial_lag(reference: SignalSeries, candidate: SignalSeries) -> dict[str, Any]:
    grid_rate_hz = min(reference.sample_rate_hz, candidate.sample_rate_hz)
    grid_dt_sec = 1.0 / grid_rate_hz
    start_sec = max(reference.times_sec[0], candidate.times_sec[0])
    end_sec = min(reference.times_sec[-1], candidate.times_sec[-1])
    if end_sec <= start_sec:
        raise ValueError("reference/input signals do not overlap in time")

    sample_count = int(math.floor((end_sec - start_sec) / grid_dt_sec)) + 1
    if sample_count < 2:
        raise ValueError("not enough common-grid samples for cross-correlation")

    grid = start_sec + np.arange(sample_count, dtype=np.float64) * grid_dt_sec
    ref_grid = np.interp(grid, reference.times_sec, reference.values)
    cand_grid = np.interp(grid, candidate.times_sec, candidate.values)

    corr = scipy.signal.correlate(
        normalize_for_xcorr(ref_grid),
        normalize_for_xcorr(cand_grid),
        mode="full",
        method="auto",
    )
    lags = scipy.signal.correlation_lags(ref_grid.size, cand_grid.size, mode="full")
    lag_samples = int(lags[int(np.argmax(corr))])
    lag_sec = float(lag_samples * grid_dt_sec)

    return {
        "grid_rate_hz": float(grid_rate_hz),
        "grid_dt_sec": float(grid_dt_sec),
        "grid_samples": int(sample_count),
        "lag_samples": lag_samples,
        "lag_sec": lag_sec,
    }


def align_to_reference(
    reference: SignalSeries,
    candidate: SignalSeries,
    lag_sec: float,
) -> dict[str, np.ndarray]:
    shifted_times = candidate.times_sec + lag_sec
    overlap_start = max(reference.times_sec[0], shifted_times[0])
    overlap_end = min(reference.times_sec[-1], shifted_times[-1])
    if overlap_end <= overlap_start:
        raise ValueError("signals do not overlap after lag alignment")

    ref_mask = (reference.times_sec >= overlap_start) & (reference.times_sec <= overlap_end)
    ref_times = reference.times_sec[ref_mask]
    ref_values = reference.values[ref_mask]
    if ref_times.size < 2:
        raise ValueError("overlap after alignment is too short")

    aligned_values = np.interp(ref_times, shifted_times, candidate.values, left=np.nan, right=np.nan)
    valid = np.isfinite(aligned_values) & np.isfinite(ref_values)
    ref_times = ref_times[valid]
    ref_values = ref_values[valid]
    aligned_values = aligned_values[valid]
    if ref_times.size < 2:
        raise ValueError("fewer than two aligned samples remain after trimming")

    return {
        "times_sec": ref_times,
        "reference_values": ref_values,
        "input_values": aligned_values,
        "errors": aligned_values - ref_values,
    }


def fit_slope(times_sec: np.ndarray, values: np.ndarray) -> tuple[float, float]:
    if times_sec.size == 0 or values.size == 0:
        return float("nan"), float("nan")
    if times_sec.size == 1:
        return 0.0, float(values[0])
    slope, intercept = np.polyfit(times_sec, values, 1)
    return float(slope), float(intercept)


def compute_window_metrics(
    times_sec: np.ndarray,
    reference_values: np.ndarray,
    input_values: np.ndarray,
    reference_rate_hz: float,
) -> dict[str, Any]:
    base_window_samples = int(round(0.5 * reference_rate_hz))
    window_samples = max(8, base_window_samples)
    if times_sec.size < window_samples:
        window_samples = times_sec.size
    step_samples = max(1, window_samples // 2)

    lag_windows: list[dict[str, float]] = []
    amp_windows: list[dict[str, float]] = []

    start_indices = list(range(0, max(times_sec.size - window_samples + 1, 1), step_samples))
    if start_indices:
        last_start = max(times_sec.size - window_samples, 0)
        if start_indices[-1] != last_start:
            start_indices.append(last_start)
    else:
        start_indices = [0]

    for start in start_indices:
        end = start + window_samples
        ref_seg = reference_values[start:end]
        in_seg = input_values[start:end]
        time_seg = times_sec[start:end]
        if ref_seg.size < 2 or in_seg.size < 2:
            continue

        center_sec = float(np.mean(time_seg))
        lag_samples = int(
            scipy.signal.correlation_lags(ref_seg.size, in_seg.size, mode="full")[
                int(
                    np.argmax(
                        scipy.signal.correlate(
                            normalize_for_xcorr(ref_seg),
                            normalize_for_xcorr(in_seg),
                            mode="full",
                            method="auto",
                        )
                    )
                )
            ]
        )
        lag_sec = float(lag_samples / reference_rate_hz)

        ref_centered = ref_seg - float(np.mean(ref_seg))
        in_centered = in_seg - float(np.mean(in_seg))
        ref_rms = float(np.sqrt(np.mean(ref_centered**2)))
        in_rms = float(np.sqrt(np.mean(in_centered**2)))
        amplitude_ratio = float(in_rms / ref_rms) if ref_rms > 0 else float("nan")

        lag_windows.append(
            {
                "center_sec": center_sec,
                "lag_sec": lag_sec,
                "lag_samples": float(lag_samples),
            }
        )
        amp_windows.append(
            {
                "center_sec": center_sec,
                "amplitude_ratio": amplitude_ratio,
            }
        )

    lag_times = np.asarray([window["center_sec"] for window in lag_windows], dtype=np.float64)
    lag_values = np.asarray([window["lag_sec"] for window in lag_windows], dtype=np.float64)
    amp_times = np.asarray([window["center_sec"] for window in amp_windows], dtype=np.float64)
    amp_values = np.asarray(
        [window["amplitude_ratio"] for window in amp_windows], dtype=np.float64
    )

    lag_slope, lag_intercept = fit_slope(lag_times, lag_values)
    amp_slope, amp_intercept = fit_slope(amp_times, amp_values)

    return {
        "window_sec": float(window_samples / reference_rate_hz),
        "step_sec": float(step_samples / reference_rate_hz),
        "lag_windows": lag_windows,
        "amplitude_windows": amp_windows,
        "lag_sec_per_sec": lag_slope,
        "lag_intercept_sec": lag_intercept,
        "amplitude_ratio_per_sec": amp_slope,
        "amplitude_ratio_intercept": amp_intercept,
    }


def compute_summary(
    reference: SignalSeries,
    input_signal: SignalSeries,
    input_format: str,
    signal_column: str,
    lag_info: dict[str, Any],
    aligned: dict[str, np.ndarray],
) -> dict[str, Any]:
    errors = aligned["errors"]
    abs_errors = np.abs(errors)
    rel_errors = relative_errors(errors, aligned["reference_values"])

    correlation = float(np.corrcoef(aligned["reference_values"], aligned["input_values"])[0, 1])
    drift = compute_window_metrics(
        aligned["times_sec"],
        aligned["reference_values"],
        aligned["input_values"],
        reference.sample_rate_hz,
    )

    return {
        "reference": {
            "name": reference.name,
            "samples": reference.samples,
            "duration_sec": reference.duration_sec,
            "dt_sec": reference.dt_sec,
            "sample_rate_hz": reference.sample_rate_hz,
        },
        "input": {
            "name": input_signal.name,
            "format": input_format,
            "signal_column": signal_column,
            "samples": input_signal.samples,
            "duration_sec": input_signal.duration_sec,
            "dt_sec": input_signal.dt_sec,
            "sample_rate_hz": input_signal.sample_rate_hz,
        },
        "alignment": {
            "common_grid_rate_hz": lag_info["grid_rate_hz"],
            "common_grid_dt_sec": lag_info["grid_dt_sec"],
            "common_grid_samples": lag_info["grid_samples"],
            "initial_lag_sec": lag_info["lag_sec"],
            "initial_lag_samples": lag_info["lag_samples"],
        },
        "overlap": {
            "samples": int(aligned["times_sec"].size),
            "duration_sec": float(aligned["times_sec"][-1] - aligned["times_sec"][0]),
            "start_sec": float(aligned["times_sec"][0]),
            "end_sec": float(aligned["times_sec"][-1]),
        },
        "metrics": {
            "rmse": float(np.sqrt(np.mean(errors**2))),
            "mae": float(np.mean(abs_errors)),
            "max_abs_error": float(np.max(abs_errors)),
            "pearson_correlation": correlation,
            "mean_error": float(np.mean(errors)),
            "std_error": float(np.std(errors)),
            "min_error": float(np.min(errors)),
            "max_error": float(np.max(errors)),
            "abs_error_p50": float(np.percentile(abs_errors, 50)),
            "abs_error_p95": float(np.percentile(abs_errors, 95)),
            "abs_error_p99": float(np.percentile(abs_errors, 99)),
            "mean_relative_error": float(np.mean(rel_errors)),
            "relative_error_p50": float(np.percentile(rel_errors, 50)),
            "relative_error_p95": float(np.percentile(rel_errors, 95)),
            "relative_error_p99": float(np.percentile(rel_errors, 99)),
            "lag_sec_per_sec": drift["lag_sec_per_sec"],
            "amplitude_ratio_per_sec": drift["amplitude_ratio_per_sec"],
            "mean_amplitude_ratio": float(
                np.mean([row["amplitude_ratio"] for row in drift["amplitude_windows"]])
            )
            if drift["amplitude_windows"]
            else float("nan"),
        },
        "window_metrics": drift,
    }


def write_summary_csv(summary: dict[str, Any], out_path: Path) -> None:
    row = {
        "reference_name": summary["reference"]["name"],
        "reference_samples": summary["reference"]["samples"],
        "reference_dt_sec": summary["reference"]["dt_sec"],
        "reference_rate_hz": summary["reference"]["sample_rate_hz"],
        "input_name": summary["input"]["name"],
        "input_format": summary["input"]["format"],
        "signal_column": summary["input"]["signal_column"],
        "input_samples": summary["input"]["samples"],
        "input_dt_sec": summary["input"]["dt_sec"],
        "input_rate_hz": summary["input"]["sample_rate_hz"],
        "common_grid_rate_hz": summary["alignment"]["common_grid_rate_hz"],
        "initial_lag_sec": summary["alignment"]["initial_lag_sec"],
        "initial_lag_samples": summary["alignment"]["initial_lag_samples"],
        "overlap_samples": summary["overlap"]["samples"],
        "overlap_duration_sec": summary["overlap"]["duration_sec"],
        **summary["metrics"],
    }
    pd.DataFrame([row]).to_csv(out_path, index=False)


def write_aligned_samples(aligned: dict[str, np.ndarray], out_path: Path) -> None:
    df = pd.DataFrame(
        {
            "time_sec": aligned["times_sec"],
            "reference_value": aligned["reference_values"],
            "input_value_aligned": aligned["input_values"],
            "error": aligned["errors"],
            "abs_error": np.abs(aligned["errors"]),
            "relative_error": relative_errors(
                aligned["errors"],
                aligned["reference_values"],
            ),
        }
    )
    df.to_csv(out_path, index=False)


def run_comparison(
    reference_path: Path,
    reference_channel: int,
    input_path: Path,
    input_format: str,
    signal_column: str,
    out_dir: Path,
    plot: bool,
) -> dict[str, Any]:
    if not 0 <= reference_channel < REFERENCE_CHANNEL_COUNT:
        raise ValueError("reference_channel must be between 0 and 13")

    reference = load_reference_signal(reference_path, reference_channel)
    if input_format == "csv":
        input_signal = load_csv_signal(input_path, reference_channel, signal_column)
    elif input_format == "parquet":
        input_signal = load_parquet_signal(input_path, signal_column)
    else:
        raise ValueError(f"unsupported input_format {input_format!r}")

    lag_info = estimate_initial_lag(reference, input_signal)
    aligned = align_to_reference(reference, input_signal, lag_info["lag_sec"])
    summary = compute_summary(
        reference,
        input_signal,
        input_format,
        signal_column,
        lag_info,
        aligned,
    )

    out_dir.mkdir(parents=True, exist_ok=True)
    summary_json_path = out_dir / "summary.json"
    summary_csv_path = out_dir / "summary.csv"
    aligned_path = out_dir / "aligned_samples.csv"

    with summary_json_path.open("w", encoding="utf-8") as handle:
        json.dump(summary, handle, indent=2, sort_keys=True)
        handle.write("\n")
    write_summary_csv(summary, summary_csv_path)
    write_aligned_samples(aligned, aligned_path)
    if plot:
        render_plots(out_dir, reference, aligned, summary)

    summary["artifacts"] = {
        "summary_json": str(summary_json_path),
        "summary_csv": str(summary_csv_path),
        "aligned_samples": str(aligned_path),
    }
    if plot:
        summary["artifacts"].update(
            {
                "overlay_png": str(out_dir / "overlay.png"),
                "error_trace_png": str(out_dir / "error_trace.png"),
                "lag_drift_png": str(out_dir / "lag_drift.png"),
                "amplitude_drift_png": str(out_dir / "amplitude_drift.png"),
            }
        )

    return summary


THRESHOLD_METRIC_NAMES = {
    "max_rmse": ("metrics", "rmse", "<="),
    "max_mae": ("metrics", "mae", "<="),
    "max_abs_error": ("metrics", "max_abs_error", "<="),
    "max_max_abs_error": ("metrics", "max_abs_error", "<="),
    "max_abs_error_p95": ("metrics", "abs_error_p95", "<="),
    "max_abs_error_p99": ("metrics", "abs_error_p99", "<="),
    "max_mean_relative_error": ("metrics", "mean_relative_error", "<="),
    "max_relative_error_p95": ("metrics", "relative_error_p95", "<="),
    "max_relative_error_p99": ("metrics", "relative_error_p99", "<="),
    "min_pearson_correlation": ("metrics", "pearson_correlation", ">="),
    "max_abs_initial_lag_sec": ("alignment", "initial_lag_sec", "abs<="),
    "max_abs_lag_sec_per_sec": ("metrics", "lag_sec_per_sec", "abs<="),
    "max_abs_amplitude_ratio_per_sec": (
        "metrics",
        "amplitude_ratio_per_sec",
        "abs<=",
    ),
}


def evaluate_thresholds(summary: dict[str, Any], thresholds: dict[str, Any]) -> list[dict[str, Any]]:
    checks: list[dict[str, Any]] = []
    for name, limit_raw in thresholds.items():
        if name not in THRESHOLD_METRIC_NAMES:
            raise ValueError(f"unknown threshold {name!r}")
        section, metric_name, op = THRESHOLD_METRIC_NAMES[name]
        value = float(summary[section][metric_name])
        limit = float(limit_raw)
        comparable = abs(value) if op == "abs<=" else value
        passed = comparable >= limit if op == ">=" else comparable <= limit
        checks.append(
            {
                "threshold": name,
                "metric": f"{section}.{metric_name}",
                "op": op,
                "value": value,
                "limit": limit,
                "passed": bool(passed),
            }
        )
    return checks


def resolve_config_path(config_dir: Path, value: str | Path) -> Path:
    path = Path(value)
    if path.is_absolute():
        return path
    return (config_dir / path).resolve()


def merged_thresholds(
    defaults: dict[str, Any],
    case: dict[str, Any],
) -> dict[str, Any]:
    merged = dict(defaults.get("thresholds", {}))
    merged.update(case.get("thresholds", {}))
    return merged


def write_suite_reports(out_dir: Path, results: list[dict[str, Any]]) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    suite_json = out_dir / "suite_summary.json"
    with suite_json.open("w", encoding="utf-8") as handle:
        json.dump(
            {
                "passed": all(row["passed"] for row in results),
                "case_count": len(results),
                "results": results,
            },
            handle,
            indent=2,
            sort_keys=True,
        )
        handle.write("\n")

    rows = []
    for result in results:
        summary = result.get("summary", {})
        metrics = summary.get("metrics", {})
        alignment = summary.get("alignment", {})
        rows.append(
            {
                "name": result["name"],
                "passed": result["passed"],
                "reference": result["reference"],
                "input": result["input"],
                "input_format": result["input_format"],
                "rmse": metrics.get("rmse"),
                "mae": metrics.get("mae"),
                "max_abs_error": metrics.get("max_abs_error"),
                "abs_error_p95": metrics.get("abs_error_p95"),
                "mean_relative_error": metrics.get("mean_relative_error"),
                "relative_error_p95": metrics.get("relative_error_p95"),
                "pearson_correlation": metrics.get("pearson_correlation"),
                "initial_lag_sec": alignment.get("initial_lag_sec"),
                "lag_sec_per_sec": metrics.get("lag_sec_per_sec"),
                "amplitude_ratio_per_sec": metrics.get("amplitude_ratio_per_sec"),
                "failed_checks": ",".join(
                    check["threshold"]
                    for check in result.get("checks", [])
                    if not check["passed"]
                ),
                "error": result.get("error", ""),
            }
        )
    pd.DataFrame(rows).to_csv(out_dir / "suite_summary.csv", index=False)


def cmd_run_suite(args: argparse.Namespace) -> int:
    config_path = args.config.resolve()
    with config_path.open("r", encoding="utf-8") as handle:
        config = json.load(handle)

    config_dir = config_path.parent
    defaults = config.get("defaults", {})
    out_dir = args.out_dir or config.get("output_dir")
    if out_dir is None:
        out_dir = config_dir / "results" / "suite"
    else:
        out_dir = resolve_config_path(config_dir, out_dir)

    cases = config.get("cases", [])
    if not isinstance(cases, list) or not cases:
        raise ValueError("suite config must contain a non-empty 'cases' array")

    results: list[dict[str, Any]] = []
    for index, case in enumerate(cases, start=1):
        name = str(case.get("name") or f"case_{index:02d}")
        case_out_dir = out_dir / name
        input_format = str(case.get("input_format", defaults.get("input_format", "csv")))
        signal_column = str(case.get("signal_column", defaults.get("signal_column", "raw_value")))
        plot = bool(args.plot or case.get("plot", defaults.get("plot", False)))
        thresholds = merged_thresholds(defaults, case)
        reference = resolve_config_path(config_dir, case["reference"])
        input_path = resolve_config_path(config_dir, case["input"])
        reference_channel = int(case.get("reference_channel", defaults.get("reference_channel", 0)))

        try:
            summary = run_comparison(
                reference,
                reference_channel,
                input_path,
                input_format,
                signal_column,
                case_out_dir,
                plot,
            )
            checks = evaluate_thresholds(summary, thresholds)
            passed = all(check["passed"] for check in checks)
            results.append(
                {
                    "name": name,
                    "passed": passed,
                    "reference": str(reference),
                    "reference_channel": reference_channel,
                    "input": str(input_path),
                    "input_format": input_format,
                    "signal_column": signal_column,
                    "out_dir": str(case_out_dir),
                    "checks": checks,
                    "summary": summary,
                }
            )
        except Exception as exc:  # noqa: BLE001
            results.append(
                {
                    "name": name,
                    "passed": False,
                    "reference": str(reference),
                    "reference_channel": reference_channel,
                    "input": str(input_path),
                    "input_format": input_format,
                    "signal_column": signal_column,
                    "out_dir": str(case_out_dir),
                    "checks": [],
                    "error": str(exc),
                }
            )

    write_suite_reports(out_dir, results)

    failed = [row for row in results if not row["passed"]]
    for row in results:
        status = "PASS" if row["passed"] else "FAIL"
        print(f"{status} {row['name']}")
        if "error" in row:
            print(f"  error: {row['error']}")
        else:
            failed_checks = [check for check in row["checks"] if not check["passed"]]
            for check in failed_checks:
                print(
                    "  failed "
                    f"{check['threshold']}: {check['value']:.12g} "
                    f"{check['op']} {check['limit']:.12g}"
                )

    print(f"suite_summary_json: {out_dir / 'suite_summary.json'}")
    print(f"suite_summary_csv: {out_dir / 'suite_summary.csv'}")
    return 1 if failed else 0


def render_plots(
    out_dir: Path,
    reference: SignalSeries,
    aligned: dict[str, np.ndarray],
    summary: dict[str, Any],
) -> None:
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt

    lag_windows = summary["window_metrics"]["lag_windows"]
    amp_windows = summary["window_metrics"]["amplitude_windows"]

    fig, ax = plt.subplots(figsize=(12, 5))
    ax.plot(aligned["times_sec"], aligned["reference_values"], label="reference", linewidth=1.0)
    ax.plot(
        aligned["times_sec"],
        aligned["input_values"],
        label="input_aligned",
        linewidth=1.0,
        alpha=0.85,
    )
    ax.set_title("Aligned Signal Overlay")
    ax.set_xlabel("Time (s)")
    ax.set_ylabel("Signal")
    ax.legend()
    fig.tight_layout()
    fig.savefig(out_dir / "overlay.png", dpi=150)
    plt.close(fig)

    fig, ax = plt.subplots(figsize=(12, 4))
    ax.plot(aligned["times_sec"], aligned["errors"], linewidth=0.9)
    ax.set_title("Pointwise Error Trace")
    ax.set_xlabel("Time (s)")
    ax.set_ylabel("Error")
    fig.tight_layout()
    fig.savefig(out_dir / "error_trace.png", dpi=150)
    plt.close(fig)

    fig, ax = plt.subplots(figsize=(12, 4))
    if lag_windows:
        lag_times = np.asarray([row["center_sec"] for row in lag_windows], dtype=np.float64)
        lag_values = np.asarray([row["lag_sec"] for row in lag_windows], dtype=np.float64)
        slope = summary["metrics"]["lag_sec_per_sec"]
        intercept = summary["window_metrics"]["lag_intercept_sec"]
        ax.plot(lag_times, lag_values, marker="o", linewidth=0.9, markersize=3)
        if lag_times.size >= 1 and np.isfinite(slope) and np.isfinite(intercept):
            ax.plot(lag_times, slope * lag_times + intercept, linestyle="--", linewidth=1.0)
    ax.set_title("Lag Drift")
    ax.set_xlabel("Time (s)")
    ax.set_ylabel("Lag (s)")
    fig.tight_layout()
    fig.savefig(out_dir / "lag_drift.png", dpi=150)
    plt.close(fig)

    fig, ax = plt.subplots(figsize=(12, 4))
    if amp_windows:
        amp_times = np.asarray([row["center_sec"] for row in amp_windows], dtype=np.float64)
        amp_values = np.asarray(
            [row["amplitude_ratio"] for row in amp_windows], dtype=np.float64
        )
        slope = summary["metrics"]["amplitude_ratio_per_sec"]
        intercept = summary["window_metrics"]["amplitude_ratio_intercept"]
        ax.plot(amp_times, amp_values, marker="o", linewidth=0.9, markersize=3)
        if amp_times.size >= 1 and np.isfinite(slope) and np.isfinite(intercept):
            ax.plot(amp_times, slope * amp_times + intercept, linestyle="--", linewidth=1.0)
    ax.set_title("Amplitude Drift")
    ax.set_xlabel("Time (s)")
    ax.set_ylabel("Amplitude Ratio")
    fig.tight_layout()
    fig.savefig(out_dir / "amplitude_drift.png", dpi=150)
    plt.close(fig)


def parse_channels_csv(text: str) -> list[int]:
    channels: list[int] = []
    for part in text.split(","):
        part = part.strip()
        if not part:
            continue
        channel = int(part)
        if channel < 0:
            raise ValueError(f"invalid channel {channel}")
        channels.append(channel)
    if not channels:
        raise ValueError("no channels provided")
    return channels


def json_dumps(payload: dict[str, Any]) -> str:
    return json.dumps(payload, separators=(",", ":"), sort_keys=False)


def cmd_inspect_ref(args: argparse.Namespace) -> None:
    table = load_reference_table(args.reference)
    available = ",".join(str(ch) for ch in table["available_channels"])
    print(f"reference: {args.reference}")
    print(f"rows: {table['rows']}")
    print(f"row_width: {table['row_width']}")
    print(f"effective_dt_sec: {table['dt_sec']:.12f}")
    print(f"effective_sample_rate_hz: {table['sample_rate_hz']:.6f}")
    print(f"available_channels: {available}")


async def cmd_fetch_export(args: argparse.Namespace) -> None:
    request = {
        "asset": args.asset,
        "channels": parse_channels_csv(args.channels),
        "start": args.start,
        "end": args.end,
        "format": "csv",
    }

    args.out.parent.mkdir(parents=True, exist_ok=True)
    meta_frame: dict[str, Any] | None = None
    summary_frame: dict[str, Any] | None = None
    total_bytes = 0

    async with websockets.connect(args.ws, max_size=None) as ws:
        await ws.send(json_dumps(request))
        with args.out.open("wb") as handle:
            async for frame in ws:
                if isinstance(frame, bytes):
                    handle.write(frame)
                    total_bytes += len(frame)
                    continue

                payload = json.loads(frame)
                frame_type = payload.get("type")
                if frame_type == "error":
                    raise RuntimeError(payload.get("message", "exporter returned an error"))
                if frame_type == "meta":
                    meta_frame = payload
                    continue
                if frame_type == "summary":
                    summary_frame = payload
                    continue
                if frame_type == "complete":
                    break

    print(f"out: {args.out}")
    print(f"bytes_written: {total_bytes}")
    if meta_frame is not None:
        print(f"meta: {json.dumps(meta_frame, sort_keys=True)}")
    if summary_frame is not None:
        print(f"summary: {json.dumps(summary_frame, sort_keys=True)}")


def cmd_compare(args: argparse.Namespace) -> None:
    summary = run_comparison(
        args.reference,
        args.reference_channel,
        args.input,
        args.input_format,
        args.signal_column,
        args.out_dir,
        args.plot,
    )

    print(f"summary_json: {summary['artifacts']['summary_json']}")
    print(f"summary_csv: {summary['artifacts']['summary_csv']}")
    print(f"aligned_samples: {summary['artifacts']['aligned_samples']}")
    print(f"rmse: {summary['metrics']['rmse']:.12g}")
    print(f"mae: {summary['metrics']['mae']:.12g}")
    print(f"pearson_correlation: {summary['metrics']['pearson_correlation']:.12g}")
    print(f"initial_lag_sec: {summary['alignment']['initial_lag_sec']:.12g}")
    print(f"lag_sec_per_sec: {summary['metrics']['lag_sec_per_sec']:.12g}")
    print(
        "amplitude_ratio_per_sec: "
        f"{summary['metrics']['amplitude_ratio_per_sec']:.12g}"
    )


if __name__ == "__main__":
    raise SystemExit(main())
