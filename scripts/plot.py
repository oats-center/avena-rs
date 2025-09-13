import os
import glob
from datetime import timedelta

import numpy as np
import pandas as pd
import matplotlib.pyplot as plt
import matplotlib.dates as mdates

try:
    from scipy.signal import savgol_filter
    HAVE_SG = True
except Exception:
    HAVE_SG = False

OUTPUT_DIR = os.getenv("OUTPUT_DIR", "../rust-ljm/outputs")
OUTPUT_FILE = os.getenv("OUTPUT_FILE", "sample_plot.png")
PLOT_MODE = os.getenv("PLOT_MODE", "mean")
RATE_HZ = float(os.getenv("RATE_HZ", "0"))
SMOOTH_WINDOW = int(os.getenv("SMOOTH_WINDOW", "11"))
SMOOTH_POLY = int(os.getenv("SMOOTH_POLY", "3"))


def parse_values(s: str) -> np.ndarray:
    return np.array([float(x) for x in str(s).split(";") if x != ""], dtype=float)


def smooth_array(y: np.ndarray) -> np.ndarray:
    if HAVE_SG and len(y) >= SMOOTH_WINDOW and SMOOTH_WINDOW % 2 == 1 and SMOOTH_POLY < SMOOTH_WINDOW:
        try:
            return savgol_filter(y, window_length=SMOOTH_WINDOW, polyorder=SMOOTH_POLY)
        except Exception:
            return y
    return y


def expand_batches(df: pd.DataFrame, rate_hz: float):
    t_list, y_list = [], []
    dt = 1.0 / rate_hz
    for _, row in df.iterrows():
        ts0 = row["timestamp"]
        vec = row["vec"]
        for k, v in enumerate(vec):
            t_list.append(ts0 + timedelta(seconds=k * dt))
            y_list.append(v)
    return pd.to_datetime(t_list), np.asarray(y_list, dtype=float)


def process_csv(path: str, mode: str, rate_hz: float):
    df = pd.read_csv(path)
    if "timestamp" not in df.columns or "values" not in df.columns:
        return None, None
    df["timestamp"] = pd.to_datetime(df["timestamp"], errors="coerce")
    df = df.dropna(subset=["timestamp"])
    df["vec"] = df["values"].apply(parse_values)
    if mode.lower() == "expand" and rate_hz > 0:
        t, y = expand_batches(df, rate_hz)
    else:
        t = df["timestamp"].values
        y = df["vec"].apply(np.mean).to_numpy()
    return t, smooth_array(y)


def build_plot(csv_paths: list, out_path: str, mode: str, rate_hz: float):
    plt.figure(figsize=(14, 7))
    for path in csv_paths:
        base = os.path.splitext(os.path.basename(path))[0]
        label = base.split("_")[-1] if "_ch" in base else base
        t, y = process_csv(path, mode, rate_hz)
        if t is not None and y is not None:
            plt.plot(t, y, label=label)
    plt.xlabel("Time")
    plt.ylabel("Value")
    plt.title("All Channels")
    plt.legend(ncol=4, fontsize=9)
    plt.grid(True)
    plt.gca().xaxis.set_major_formatter(mdates.DateFormatter("%H:%M:%S"))
    plt.gcf().autofmt_xdate()
    plt.tight_layout()
    plt.savefig(out_path, dpi=140)
    plt.close()


def main():
    csv_paths = sorted(glob.glob(os.path.join(OUTPUT_DIR, "labjack_*_ch*.csv")))
    if not csv_paths:
        raise SystemExit(f"No per-channel CSVs found in '{OUTPUT_DIR}'")
    out_path = os.path.join(".", OUTPUT_FILE)
    build_plot(csv_paths, out_path, PLOT_MODE, RATE_HZ)
    print(f"Saved combined plot: {out_path}")


if __name__ == "__main__":
    main()
