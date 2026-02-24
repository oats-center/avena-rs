import os
import csv
import json
import time
import subprocess
import threading
from pathlib import Path
from collections import defaultdict

import numpy as np
import cv2
from sort import Sort

# =========================
# CONFIG
# =========================
RTSP_URL = "rtsp://admin:1ubuntu9@@localhost:8554"  # one camera

LANES_JSON = "lanes.json"
DRAW_LANE_DEBUG = True

X_CROSS_RATIO = 0.75                 # crossing line position (as fraction of width)
FOV_METERS_ACROSS_WIDTH = 30.0       # mph calibration assumption

KF_MEAS_STD = 10.0
KF_ACC_STD = 25.0
MAX_MISSED = 30

MIN_CONTOUR_AREA = 8000
FLOW_MAG_THRESH = 1.5

# =========================
# Run directory (like your ref)
# =========================
def increment_path(base_dir: str) -> str:
    base = Path(base_dir)
    parent = base.parent
    stem = base.name

    if not base.exists():
        base.mkdir(parents=True, exist_ok=True)
        return str(base)

    dirs = [d for d in parent.glob(f"{stem}*") if d.is_dir()]
    indices = []
    for d in dirs:
        suf = d.name.replace(stem, "")
        if suf.isdigit():
            indices.append(int(suf))
        elif suf == "":
            indices.append(0)

    next_idx = (max(indices) + 1) if indices else 1
    new_dir = parent / (stem if next_idx == 0 else f"{stem}{next_idx}")
    while new_dir.exists():
        next_idx += 1
        new_dir = parent / f"{stem}{next_idx}"

    new_dir.mkdir(parents=True, exist_ok=True)
    return str(new_dir)

SAVE_DIR = increment_path("runs/exp")
SNAP_DIR = os.path.join(SAVE_DIR, "snapshots")
os.makedirs(SNAP_DIR, exist_ok=True)
CSV_PATH = os.path.join(SAVE_DIR, "crossings.csv")

# =========================
# 1D Constant-Velocity KF (correct update)
# state: [x, vx]
# meas : x
# =========================
class KF1DConstantVel:
    def __init__(
        self,
        dt,
        meas_noise_std=10.0,
        accel_noise_std=25.0,
        init_pos_std=60.0,
        init_vel_std=80.0,
    ):
        self.dt = float(dt)
        dt = self.dt

        self.F = np.array([[1.0, dt],
                           [0.0, 1.0]], dtype=np.float64)
        self.H = np.array([[1.0, 0.0]], dtype=np.float64)

        q = float(accel_noise_std) ** 2
        self.Q = np.array([[0.25 * dt**4, 0.5 * dt**3],
                           [0.5  * dt**3,       dt**2]], dtype=np.float64) * q

        r = float(meas_noise_std) ** 2
        self.R = np.array([[r]], dtype=np.float64)

        self.P = np.diag([init_pos_std**2, init_vel_std**2]).astype(np.float64)
        self.I = np.eye(2, dtype=np.float64)

        self.x = None  # 2x1

    def _rebuild_mats_for_dt(self, dt, accel_noise_std):
        self.dt = float(dt)
        dt = self.dt
        self.F = np.array([[1.0, dt],
                           [0.0, 1.0]], dtype=np.float64)
        q = float(accel_noise_std) ** 2
        self.Q = np.array([[0.25 * dt**4, 0.5 * dt**3],
                           [0.5  * dt**3,       dt**2]], dtype=np.float64) * q

    def update(self, z_x, dt=None, accel_noise_std=25.0):
        if dt is not None:
            self._rebuild_mats_for_dt(dt, accel_noise_std)

        z = np.array([[float(z_x)]], dtype=np.float64)

        if self.x is None:
            self.x = np.zeros((2, 1), dtype=np.float64)
            self.x[0, 0] = z[0, 0]
            self._prev_z = z[0, 0]
            return self.x

        # initialize velocity from second measurement
        if abs(self.x[1, 0]) < 1e-9 and hasattr(self, "_prev_z"):
            self.x[1, 0] = (z[0, 0] - self._prev_z) / self.dt
        self._prev_z = z[0, 0]

        # predict
        self.x = self.F @ self.x
        self.P = self.F @ self.P @ self.F.T + self.Q

        # update
        y = z - (self.H @ self.x)
        S = self.H @ self.P @ self.H.T + self.R
        K = self.P @ self.H.T @ np.linalg.inv(S)
        self.x = self.x + K @ y
        self.P = (self.I - K @ self.H) @ self.P
        return self.x

    def pos(self):
        return None if self.x is None else float(self.x[0, 0])

    def vel(self):
        return None if self.x is None else float(self.x[1, 0])

# =========================
# lanes.json polygon assigner (from your ref)
# lanes.json expects:
# {
#   "image_size": {"width": W, "height": H},
#   "lanes": {"lane1": [[x,y],...], "lane2": ...}
# }
# =========================
class LanePolygonAssigner:
    def __init__(self, lanes_json_path: str, video_w: int, video_h: int):
        with open(lanes_json_path, "r") as f:
            self.data = json.load(f)

        img_w = float(self.data["image_size"]["width"])
        img_h = float(self.data["image_size"]["height"])

        self.sx = float(video_w) / img_w if img_w > 0 else 1.0
        self.sy = float(video_h) / img_h if img_h > 0 else 1.0

        lanes_dict = self.data.get("lanes", {})
        if not lanes_dict:
            raise ValueError("lanes.json has no 'lanes' field")

        def lane_key_sort(k: str) -> int:
            digits = "".join([c for c in k if c.isdigit()])
            return int(digits) if digits else 0

        self.lane_polys = []  # list of (lane_id:int, poly_np:int32 Nx1x2)
        for key in sorted(lanes_dict.keys(), key=lane_key_sort):
            pts = lanes_dict[key]  # list [[x,y],...]
            lane_id = lane_key_sort(key)

            pts_scaled = np.array(
                [[int(round(p[0] * self.sx)), int(round(p[1] * self.sy))] for p in pts],
                dtype=np.int32
            ).reshape((-1, 1, 2))

            self.lane_polys.append((lane_id, pts_scaled))

        if len(self.lane_polys) == 0:
            raise ValueError("No lane polygons found in lanes.json")

    def lane_id_from_point(self, cx: float, cy: float) -> int:
        x = float(cx); y = float(cy)
        for lane_id, poly in self.lane_polys:
            if cv2.pointPolygonTest(poly, (x, y), False) >= 0:
                return int(lane_id)
        return -1

    def draw_debug(self, im0, thickness=2):
        for lane_id, poly in self.lane_polys:
            cv2.polylines(im0, [poly], isClosed=True, color=(0, 255, 0), thickness=thickness)
            p0 = poly[0, 0]
            cv2.putText(im0, f"lane{lane_id}", (int(p0[0]), int(p0[1])),
                        cv2.FONT_HERSHEY_SIMPLEX, 0.8, (0, 255, 0), 2)

# =========================
# FFmpeg pipe helpers
# =========================
def create_ffmpeg_process(url):
    return subprocess.Popen([
        "ffmpeg", "-rtsp_transport", "tcp", "-i", url,
        "-an", "-vf", "scale=1280:-1", "-r", "20",
        "-f", "image2pipe", "-vcodec", "mjpeg", "-"
    ], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, bufsize=10**8)

def read_frame(pipe):
    data = b''
    while True:
        byte = pipe.read(1)
        if not byte:
            return None
        data += byte
        if data.endswith(b'\xff\xd9'):
            return data

def save_snapshot(frame_bgr, bbox_xyxy, car_id, t_sec):
    h, w = frame_bgr.shape[:2]
    x1, y1, x2, y2 = [int(v) for v in bbox_xyxy]
    x1 = max(0, x1); y1 = max(0, y1)
    x2 = min(w - 1, x2); y2 = min(h - 1, y2)
    if x2 <= x1 or y2 <= y1:
        return ""
    crop = frame_bgr[y1:y2, x1:x2].copy()
    out = os.path.join(SNAP_DIR, f"id{int(car_id)}_t{t_sec:.3f}.jpg")
    cv2.imwrite(out, crop)
    return out

# =========================
# MAIN
# =========================
proc = create_ffmpeg_process(RTSP_URL)

# CSV
csv_file = open(CSV_PATH, "w", newline="")
csv_writer = csv.writer(csv_file)
csv_writer.writerow([
    "car_id", "lane_id", "timestamp_s",
    "x_cross_px", "vx_px_s", "speed_mph", "snapshot_path"
])
csv_file.flush()

# Tracker + state
tracker = Sort(max_age=30, min_hits=3, iou_threshold=0.3)
kf_by_id = {}
missed_by_id = defaultdict(int)
prev_front_by_id = {}   # tid -> (front_x, t_sec)
crossed_ids = set()

# first frame init
first = read_frame(proc.stdout)
if first is None:
    raise RuntimeError("Failed to read first frame from RTSP/ffmpeg.")

frame0 = cv2.imdecode(np.frombuffer(first, dtype=np.uint8), cv2.IMREAD_COLOR)
if frame0 is None:
    raise RuntimeError("Failed to decode first frame.")

frame0 = cv2.resize(frame0, (frame0.shape[1] // 2, frame0.shape[0] // 2))
H, W = frame0.shape[:2]

x_cross = int(W * X_CROSS_RATIO)
meters_per_pixel = float(FOV_METERS_ACROSS_WIDTH) / float(W)
pxps_to_mph = meters_per_pixel * 2.2369362920544

lane_assigner = LanePolygonAssigner(LANES_JSON, W, H)

prev_gray = cv2.cvtColor(cv2.GaussianBlur(frame0, (5, 5), 0), cv2.COLOR_BGR2GRAY)

last_t = time.time()
dt_fallback = 1.0 / 20.0  # matches ffmpeg -r 20

print(f"[INFO] SAVE_DIR: {SAVE_DIR}")
print(f"[INFO] CSV:      {CSV_PATH}")
print(f"[INFO] SNAP_DIR:  {SNAP_DIR}")
print(f"[INFO] x_cross:   {x_cross}px ({int(X_CROSS_RATIO*100)}%)")
print(f"[INFO] lanes.json loaded (sx={lane_assigner.sx:.4f}, sy={lane_assigner.sy:.4f})")

try:
    while True:
        jb = read_frame(proc.stdout)
        if jb is None:
            continue

        now = time.time()
        dt = max(1e-3, now - last_t) if last_t is not None else dt_fallback
        last_t = now

        frame = cv2.imdecode(np.frombuffer(jb, dtype=np.uint8), cv2.IMREAD_COLOR)
        if frame is None:
            continue

        frame = cv2.resize(frame, (frame.shape[1] // 2, frame.shape[0] // 2))
        blurred = cv2.GaussianBlur(frame, (5, 5), 0)
        gray = cv2.cvtColor(blurred, cv2.COLOR_BGR2GRAY)

        # Optical flow motion detection
        flow = cv2.calcOpticalFlowFarneback(prev_gray, gray, None,
                                            0.5, 3, 15, 3, 5, 1.2, 0)
        mag, _ = cv2.cartToPolar(flow[..., 0], flow[..., 1])
        motion_mask = cv2.threshold(mag, FLOW_MAG_THRESH, 255, cv2.THRESH_BINARY)[1].astype(np.uint8)

        kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (10, 10))
        motion_mask = cv2.morphologyEx(motion_mask, cv2.MORPH_CLOSE, kernel)
        motion_mask = cv2.dilate(motion_mask, kernel, iterations=1)

        contours, _ = cv2.findContours(motion_mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)

        detections = []
        for contour in contours:
            area = cv2.contourArea(contour)
            if area < MIN_CONTOUR_AREA:
                continue
            x, y, w, h = cv2.boundingRect(contour)
            detections.append([x, y, x + w, y + h, 1.0])

        # SORT update
        if detections:
            tracked = tracker.update(np.array(detections))
        else:
            tracked = tracker.update(np.empty((0, 5)))

        seen = set()

        # optional debug overlay for lanes
        if DRAW_LANE_DEBUG:
            lane_assigner.draw_debug(frame, thickness=2)

        # crossing line
        cv2.line(frame, (x_cross, 0), (x_cross, H), (0, 255, 255), 2)
        cv2.putText(frame, f"x={x_cross}px", (x_cross + 10, 30),
                    cv2.FONT_HERSHEY_SIMPLEX, 0.7, (0, 255, 255), 2)

        for *bbox, obj_id in tracked:
            tid = int(obj_id)
            x1, y1, x2, y2 = map(float, bbox)
            seen.add(tid)

            cx = 0.5 * (x1 + x2)
            cy = 0.5 * (y1 + y2)
            lane_id = lane_assigner.lane_id_from_point(cx, cy)

            # KF on FRONT edge (x2) for left->right
            if tid not in kf_by_id:
                kf_by_id[tid] = KF1DConstantVel(
                    dt=dt,
                    meas_noise_std=KF_MEAS_STD,
                    accel_noise_std=KF_ACC_STD,
                    init_pos_std=60.0,
                    init_vel_std=80.0,
                )
            kf = kf_by_id[tid]
            front_x = float(x2)
            kf.update(front_x, dt=dt, accel_noise_std=KF_ACC_STD)

            vx = kf.vel()
            speed_mph = (abs(vx) * pxps_to_mph) if vx is not None else 0.0

            missed_by_id[tid] = 0

            # crossing event
            if tid not in crossed_ids:
                prev = prev_front_by_id.get(tid)
                if prev is not None:
                    prev_front, prev_t = prev
                    if prev_front < x_cross <= front_x:  # left -> right
                        alpha = (x_cross - prev_front) / max(1e-6, (front_x - prev_front))
                        t_cross = prev_t + alpha * (now - prev_t)

                        snap = save_snapshot(frame, (x1, y1, x2, y2), tid, t_cross)

                        csv_writer.writerow([
                            tid,
                            int(lane_id),
                            f"{t_cross:.6f}",
                            f"{float(x_cross):.2f}",
                            f"{vx:.3f}" if vx is not None else "",
                            f"{speed_mph:.3f}",
                            snap,
                        ])
                        csv_file.flush()
                        crossed_ids.add(tid)

                prev_front_by_id[tid] = (front_x, now)
            else:
                prev_front_by_id[tid] = (front_x, now)

            # draw
            x1i, y1i, x2i, y2i = map(int, [x1, y1, x2, y2])
            cv2.rectangle(frame, (x1i, y1i), (x2i, y2i), (0, 255, 0), 2)
            cv2.circle(frame, (int(cx), int(cy)), 4, (0, 0, 255), -1)
            cv2.putText(frame,
                        f"ID {tid} lane {lane_id} {speed_mph:.1f}mph",
                        (x1i, max(0, y1i - 10)),
                        cv2.FONT_HERSHEY_SIMPLEX, 0.6, (255, 0, 0), 2)

        # missed + cleanup
        for tid in list(kf_by_id.keys()):
            if tid not in seen:
                missed_by_id[tid] += 1
                if missed_by_id[tid] > MAX_MISSED:
                    kf_by_id.pop(tid, None)
                    missed_by_id.pop(tid, None)
                    prev_front_by_id.pop(tid, None)
                    crossed_ids.discard(tid)

        cv2.imshow("Single Camera Tracking", frame)
        if cv2.waitKey(1) == 27:  # ESC
            break

        prev_gray = gray.copy()

finally:
    proc.terminate()
    csv_file.close()
    cv2.destroyAllWindows()
    print(f"[INFO] Done. Saved: {SAVE_DIR}")