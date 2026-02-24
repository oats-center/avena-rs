import os
import csv
import math
import cv2
import numpy as np
from pathlib import Path
from typing import Optional, Tuple, List

from ultralytics import YOLO


def safe_int(x, default=-1):
    try:
        return int(x)
    except Exception:
        return default


def safe_float(x, default=0.0):
    try:
        return float(x)
    except Exception:
        return default


def clamp(v, lo, hi):
    return max(lo, min(hi, v))


def ensure_dir(p: str) -> str:
    os.makedirs(p, exist_ok=True)
    return p


def resolve_class_id(names, class_id, class_name: str) -> int:
    if class_id is not None:
        return int(class_id)

    if isinstance(names, dict):
        name_to_id = {str(v): int(k) for k, v in names.items()}
    else:
        name_to_id = {str(v): i for i, v in enumerate(names)}

    if class_name in name_to_id:
        return int(name_to_id[class_name])

    print(f"[WARN] Class '{class_name}' not found in model.names. Falling back to id=0.")
    return 0


def cluster_1d_sorted(values_sorted: List[float], eps: float) -> List[List[float]]:
    if not values_sorted:
        return []
    clusters = []
    cur = [values_sorted[0]]
    for v in values_sorted[1:]:
        if abs(v - cur[-1]) <= eps:
            cur.append(v)
        else:
            clusters.append(cur)
            cur = [v]
    clusters.append(cur)
    return clusters

def median(xs: List[float]) -> float:
    xs = sorted(xs)
    n = len(xs)
    if n == 0:
        return 0.0
    if n % 2 == 1:
        return xs[n // 2]
    return 0.5 * (xs[n // 2 - 1] + xs[n // 2])

class WheelCrossingFromCarCSV:
    """
    - reads crossings.csv from car code
    - for each event, runs on only 10 frames BEFORE t_front
    - wheel detection is ONLY inside selected car bbox
    """

    def __init__(
        self,
        video_path: str,
        crossings_csv: str,
        out_dir: str,
        model_path: str,

        car_class_name: str = "car",
        car_class_id: Optional[int] = None,

        wheel_class_name: str = "wheel",
        wheel_class_id: Optional[int] = None,

        car_conf: float = 0.25,
        wheel_conf: float = 0.25,
        imgsz: Optional[int] = None,
        n_before: int = 10,

        # dx clustering + matching
        dx_cluster_eps_px: float = 35.0,
        dx_match_tol_px: float = 35.0,

        # crop padding
        car_crop_pad: int = 10,
    ):
        self.video_path = video_path
        self.crossings_csv = crossings_csv
        self.out_dir = ensure_dir(out_dir)
        self.snap_dir = ensure_dir(os.path.join(self.out_dir, "wheel_snapshots"))

        self.model = YOLO(model_path)
        self.names = self.model.names

        self.car_class_id = resolve_class_id(self.names, car_class_id, car_class_name)
        self.wheel_class_id = resolve_class_id(self.names, wheel_class_id, wheel_class_name)

        self.car_conf = float(car_conf)
        self.wheel_conf = float(wheel_conf)
        self.imgsz = imgsz

        self.n_before = int(n_before)
        self.dx_cluster_eps_px = float(dx_cluster_eps_px)
        self.dx_match_tol_px = float(dx_match_tol_px)
        self.car_crop_pad = int(car_crop_pad)

        self.cap = cv2.VideoCapture(self.video_path)
        assert self.cap.isOpened(), f"Error reading video: {video_path}"

        self.w = int(self.cap.get(cv2.CAP_PROP_FRAME_WIDTH))
        self.h = int(self.cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
        self.fps = float(self.cap.get(cv2.CAP_PROP_FPS))
        if self.fps <= 0:
            self.fps = 30.0

        self.out_csv_path = os.path.join(self.out_dir, "wheels_crossings.csv")
        self.out_f = open(self.out_csv_path, "w", newline="")
        self.out_w = csv.writer(self.out_f)
        self.out_w.writerow([
            "car_id",
            "lane_id",
            "num_wheels",
            "wheel_id",
            "dx_front_to_wheel_px",
            "t_front_cross_s",
            "vx_px_s",
            "t_wheel_cross_s",
            "snapshot_frame_idx",
            "snapshot_path",
        ])

        print(f"[INFO] Video: {video_path} ({self.w}x{self.h} @ {self.fps:.2f} fps)")
        print(f"[INFO] Model: {model_path}")
        print(f"[INFO] car_class_id={self.car_class_id}, wheel_class_id={self.wheel_class_id}")
        print(f"[INFO] Output CSV: {self.out_csv_path}")
        print(f"[INFO] Snapshots: {self.snap_dir}")

    def t_to_k(self, t: float) -> int:
        return int(round(float(t) * self.fps))

    def get_frame(self, k: int):
        k = int(k)
        if k < 0:
            return None
        self.cap.set(cv2.CAP_PROP_POS_FRAMES, k)
        ok, frame = self.cap.read()
        if not ok:
            return None
        return frame

    def detect_car_candidates(self, frame_bgr):
        """
        Returns list of (x1,y1,x2,y2, conf) for car class only.
        """
        kwargs = dict(verbose=False, conf=self.car_conf)
        if self.imgsz is not None:
            kwargs["imgsz"] = self.imgsz

        res = self.model.predict(frame_bgr, **kwargs)
        if not res:
            return []

        r0 = res[0]
        if r0.boxes is None or len(r0.boxes) == 0:
            return []

        xyxy = r0.boxes.xyxy.cpu().numpy()
        clss = r0.boxes.cls.cpu().numpy().astype(int)
        confs = r0.boxes.conf.cpu().numpy()

        cars = []
        for box, cls, cf in zip(xyxy, clss, confs):
            if cls != self.car_class_id:
                continue
            x1, y1, x2, y2 = map(float, box)
            cars.append((x1, y1, x2, y2, float(cf)))
        return cars

    def pick_car_near_xcross(self, cars, x_cross: float) -> Optional[Tuple[float, float, float, float]]:
        """
        Pick the car bbox whose front (x2) is closest to x_cross.
        """
        if not cars:
            return None
        best = None
        best_err = 1e18
        for x1, y1, x2, y2, cf in cars:
            err = abs(float(x2) - float(x_cross))
            if err < best_err:
                best_err = err
                best = (x1, y1, x2, y2)
        return best

    def detect_wheels_in_car(self, frame_bgr, car_bbox_xyxy):
        """
        Run model on car crop, return wheel boxes in FULL frame:
        list of (wx1,wy1,wx2,wy2, conf).
        """
        x1, y1, x2, y2 = car_bbox_xyxy
        pad = self.car_crop_pad
        ix1 = clamp(int(math.floor(x1)) - pad, 0, self.w - 1)
        iy1 = clamp(int(math.floor(y1)) - pad, 0, self.h - 1)
        ix2 = clamp(int(math.ceil(x2)) + pad, 0, self.w - 1)
        iy2 = clamp(int(math.ceil(y2)) + pad, 0, self.h - 1)
        if ix2 <= ix1 or iy2 <= iy1:
            return []

        crop = frame_bgr[iy1:iy2, ix1:ix2]

        kwargs = dict(verbose=False, conf=self.wheel_conf)
        if self.imgsz is not None:
            kwargs["imgsz"] = self.imgsz

        res = self.model.predict(crop, **kwargs)
        if not res:
            return []

        r0 = res[0]
        if r0.boxes is None or len(r0.boxes) == 0:
            return []

        xyxy = r0.boxes.xyxy.cpu().numpy()
        clss = r0.boxes.cls.cpu().numpy().astype(int)
        confs = r0.boxes.conf.cpu().numpy()

        wheels = []
        for box, cls, cf in zip(xyxy, clss, confs):
            if cls != self.wheel_class_id:
                continue
            wx1, wy1, wx2, wy2 = map(float, box)
            wheels.append((wx1 + ix1, wy1 + iy1, wx2 + ix1, wy2 + iy1, float(cf)))
        return wheels

    def save_wheel_snapshot(self, frame_bgr, wheel_xyxy, car_id, wheel_id, t_sec) -> str:
        wx1, wy1, wx2, wy2 = [int(round(v)) for v in wheel_xyxy]
        wx1 = clamp(wx1, 0, self.w - 1)
        wy1 = clamp(wy1, 0, self.h - 1)
        wx2 = clamp(wx2, 0, self.w - 1)
        wy2 = clamp(wy2, 0, self.h - 1)
        if wx2 <= wx1 or wy2 <= wy1:
            return ""
        crop = frame_bgr[wy1:wy2, wx1:wx2].copy()
        out_path = os.path.join(self.snap_dir, f"car{car_id}_wheel{wheel_id}_t{t_sec:.6f}.jpg")
        cv2.imwrite(out_path, crop)
        return out_path

    def process_one_event(self, car_id: int, lane_id: int, t_front: float, x_cross: float, vx: float):
        if abs(vx) < 1e-3:
            print(f"[WARN] Skip car_id={car_id} (vx too small).")
            return

        k_front = self.t_to_k(t_front)
        k0 = max(0, k_front - self.n_before)
        k1 = k_front  

        dx_all = []

        # ---- collect dx from frames k0..k1 ----
        for k in range(k0, k1 + 1):
            frame = self.get_frame(k)
            if frame is None:
                continue

            cars = self.detect_car_candidates(frame)
            car_bbox = self.pick_car_near_xcross(cars, x_cross)
            if car_bbox is None:
                continue

            front_x = float(car_bbox[2])

            wheels = self.detect_wheels_in_car(frame, car_bbox)
            for wx1, wy1, wx2, wy2, cf in wheels:
                wcx = 0.5 * (wx1 + wx2)
                dx = front_x - wcx  # positive if behind front
                if dx < -5:
                    continue
                dx_all.append(dx)

        if not dx_all:
            print(f"[WARN] No wheels detected in 10 frames before: car_id={car_id} t_front={t_front:.3f}")
            return

        # ---- cluster dx into wheel IDs ----
        dx_sorted = sorted(dx_all)
        clusters = cluster_1d_sorted(dx_sorted, eps=self.dx_cluster_eps_px)
        dx_wheels = [median(c) for c in clusters]
        dx_wheels.sort()
        num_wheels = len(dx_wheels)

        # ---- compute wheel times and snapshot each ----
        for wheel_id, dx in enumerate(dx_wheels, start=1):
            t_wheel = t_front + (dx / vx)
            k_snap = self.t_to_k(t_wheel)

            frame = self.get_frame(k_snap)
            snap_path = ""
            if frame is not None:
                cars = self.detect_car_candidates(frame)
                car_bbox = self.pick_car_near_xcross(cars, x_cross)
                if car_bbox is not None:
                    front_x = float(car_bbox[2])
                    wheels = self.detect_wheels_in_car(frame, car_bbox)

                    best_w = None
                    best_err = 1e18
                    for wx1, wy1, wx2, wy2, cf in wheels:
                        wcx = 0.5 * (wx1 + wx2)
                        dx_now = front_x - wcx
                        err = abs(dx_now - dx)
                        if err < best_err:
                            best_err = err
                            best_w = (wx1, wy1, wx2, wy2)

                    if best_w is not None and best_err <= self.dx_match_tol_px:
                        snap_path = self.save_wheel_snapshot(frame, best_w, car_id, wheel_id, t_wheel)

            self.out_w.writerow([
                int(car_id),
                int(lane_id),
                int(num_wheels),
                int(wheel_id),
                f"{dx:.3f}",
                f"{t_front:.6f}",
                f"{vx:.6f}",
                f"{t_wheel:.6f}",
                int(k_snap),
                snap_path,
            ])
            self.out_f.flush()

        print(f"[OK] car_id={car_id} wheels={num_wheels} using frames [{k0}..{k1}]")

    def run(self):
        with open(self.crossings_csv, "r", newline="") as f:
            r = csv.DictReader(f)
            for row in r:
                car_id = safe_int(row.get("car_id", -1), -1)
                lane_id = safe_int(row.get("lane_id", -1), -1)
                t_front = safe_float(row.get("timestamp_s", 0.0), 0.0)
                x_cross = safe_float(row.get("x_cross_px", 0.0), 0.0)
                vx = safe_float(row.get("vx_px_s", 0.0), 0.0)

                if car_id < 0 or x_cross <= 0:
                    continue

                self.process_one_event(car_id, lane_id, t_front, x_cross, vx)

        self.out_f.close()
        self.cap.release()
        print(f"[INFO] Done. Wrote: {self.out_csv_path}")


if __name__ == "__main__":

    VIDEO_PATH = "/Users/anuskadas/Desktop/Cars/vid1.mp4"
    CROSSINGS_CSV = "yolo26_runs/exp/crossings.csv"
    OUT_DIR = "yolo26_runs/wheel_exp"
    YOLO26_MODEL = "/Users/anuskadas/PycharmProjects/Jan25/Camera_Final/train/weights/best.pt"
    CAR_CLASS_NAME = "car"
    WHEEL_CLASS_NAME = "wheel"

    extractor = WheelCrossingFromCarCSV(
        video_path=VIDEO_PATH,
        crossings_csv=CROSSINGS_CSV,
        out_dir=OUT_DIR,
        model_path=YOLO26_MODEL,

        car_class_name=CAR_CLASS_NAME,
        wheel_class_name=WHEEL_CLASS_NAME,

        car_conf=0.25,
        wheel_conf=0.20,
        imgsz=None,

        n_before=10,
        dx_cluster_eps_px=35.0,
        dx_match_tol_px=40.0,

        car_crop_pad=10,
    )
    extractor.run()
