import os
import csv
import json
import cv2
import numpy as np
import subprocess
import time
from pathlib import Path
from collections import defaultdict

from ultralytics import YOLO
from ultralytics.utils.plotting import colors


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


# =========================
# 1D Constant-Velocity KF
# state: [x, vx]
# meas : x
# =========================
class KF1DConstantVel:
    def __init__(
        self,
        dt,
        meas_noise_std=10.0,    # px
        accel_noise_std=25.0,   # px/s^2
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

    def predict(self):
        if self.x is None:
            return None
        self.x = self.F @ self.x
        self.P = self.F @ self.P @ self.F.T + self.Q
        return self.x

    def update(self, z_x):
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

        self.predict()
        S = self.H @ self.P @ self.H.T + self.R
        K = self.P @ self.H.T @ np.linalg.inv(S)
        self.x = self.x + K @ (z - self.H @ self.x)
        self.P = (self.I - K @ self.H) @ self.P
        return self.x

    def pos(self):
        return None if self.x is None else float(self.x[0, 0])

    def vel(self):
        return None if self.x is None else float(self.x[1, 0])


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
        x = float(cx)
        y = float(cy)
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


class CarCrossingTrackerStable:
    def __init__(
        self,
        model,
        source,
        save_dir,
        output_name="car-tracking.mp4",

        # target class
        car_class_id=None,
        car_class_name="car",

        # tracking knobs
        tracker_yaml=None,     # e.g. "bytetrack.yaml" OR None for default
        conf=None,             # None = default
        iou=None,              # None = default
        imgsz=None,            # None = default

        # crossing config
        x_cross_ratio=0.75,
        fov_meters=30.0,

        # KF params
        kf_meas_std=10.0,
        kf_acc_std=25.0,
        max_missed=30,

        # trails
        trail_len=50,
        polyline_thickness=2,

        # lanes.json support
        lanes_json=None,
        draw_lane_debug=False,

        # RTSP/ffmpeg controls
        rtsp_transport="tcp",
        rtsp_scale_w=None,
        rtsp_fps=20.0,
    ):
        self.model = YOLO(model)
        self.names = self.model.names

        self.source = source
        self.use_ffmpeg_rtsp = isinstance(source, str) and source.strip().lower().startswith("rtsp://")
        self.rtsp_transport = str(rtsp_transport)
        self.rtsp_scale_w = rtsp_scale_w
        self.rtsp_fps = float(rtsp_fps)

        self.ffmpeg_proc = None
        self._frame_idx = 0
        self._start_unix = None
        self._start_monotonic = None

        if self.use_ffmpeg_rtsp:
            self.ffmpeg_proc = self._create_ffmpeg_process(self.source)
            first = self._read_rtsp_frame()
            assert first is not None, f"Error reading first frame from RTSP: {self.source}"
            self.h, self.w = first.shape[:2]
            self.fps = self.rtsp_fps if self.rtsp_fps > 0 else 20.0
            self._first_frame = first
        else:
            self.cap = cv2.VideoCapture(source)
            assert self.cap.isOpened(), f"Error reading video file: {source}"

            self.w = int(self.cap.get(cv2.CAP_PROP_FRAME_WIDTH))
            self.h = int(self.cap.get(cv2.CAP_PROP_FRAME_HEIGHT))
            self.fps = float(self.cap.get(cv2.CAP_PROP_FPS))
            if self.fps <= 0:
                self.fps = 30.0
            self._first_frame = None

        # Create new run directory every time
        self.save_dir = increment_path(save_dir)
        self.output_path = os.path.join(self.save_dir, output_name)

        fourcc = cv2.VideoWriter_fourcc(*"mp4v")
        self.writer = cv2.VideoWriter(self.output_path, fourcc, self.fps, (self.w, self.h))
        if not self.writer.isOpened():
            fourcc = cv2.VideoWriter_fourcc(*"avc1")
            self.writer = cv2.VideoWriter(self.output_path, fourcc, self.fps, (self.w, self.h))
        assert self.writer.isOpened(), f"Error opening VideoWriter at: {self.output_path}"

        # crossing line
        self.x_cross_ratio = float(x_cross_ratio)
        self.x_cross = int(self.w * self.x_cross_ratio)

        # mph conversion
        self.fov_meters = float(fov_meters)
        self.meters_per_pixel = self.fov_meters / float(self.w)
        self.PXPS_TO_MPH = self.meters_per_pixel * 2.2369362920544

        # fallback lane separators
        self.lane_y1 = self.h // 3
        self.lane_y2 = (2 * self.h) // 3

        # snapshots + CSV
        self.snap_dir = os.path.join(self.save_dir, "snapshots")
        os.makedirs(self.snap_dir, exist_ok=True)

        self.csv_path = os.path.join(self.save_dir, "crossings.csv")
        self.csv_file = open(self.csv_path, "w", newline="")
        self.csv_writer = csv.writer(self.csv_file)
        self.csv_writer.writerow([
            "car_id", "lane_id",
            "timestamp_unix",
            "x_cross_px", "vx_px_s", "speed_mph", "snapshot_path"
        ])

        # KF / state per id
        self.kf_by_id = {}
        self.last_bbox_by_id = {}
        self.last_center_by_id = {}
        self.missed_by_id = defaultdict(int)

        self.prev_front_by_id = {}
        self.crossed_ids = set()

        # trails
        self.track_history = defaultdict(list)
        self.trail_len = int(trail_len)
        self.polyline_thickness = int(polyline_thickness)

        # KF tuning
        self.kf_meas_std = float(kf_meas_std)
        self.kf_acc_std = float(kf_acc_std)
        self.max_missed = int(max_missed)

        # tracking options
        self.tracker_yaml = tracker_yaml
        self.conf = conf
        self.iou = iou
        self.imgsz = imgsz

        # pick class id
        self.car_class_name = str(car_class_name)
        self.car_class_id = self._resolve_car_class_id(car_class_id, self.car_class_name)

        # lanes.json polygon assigner
        self.lane_assigner = None
        self.draw_lane_debug = bool(draw_lane_debug)
        if lanes_json is not None:
            self.lane_assigner = LanePolygonAssigner(lanes_json, self.w, self.h)
            print(f"[INFO] Loaded lanes.json (sx={self.lane_assigner.sx:.4f}, sy={self.lane_assigner.sy:.4f})")

        # UI
        self.window_name = "Car Tracking (RTSP + KF mph + lanes.json)"
        cv2.namedWindow(self.window_name, cv2.WINDOW_NORMAL)

        print(f"[INFO] Model names: {self.names}")
        print(f"[INFO] Using car_class_id={self.car_class_id} (name='{self.car_class_name}')")
        print(f"[INFO] x_cross = {self.x_cross}px ({int(self.x_cross_ratio*100)}% of width)")
        print(f"[INFO] mph assumption: {self.fov_meters}m across width => {self.meters_per_pixel:.6f} m/px")
        print(f"[INFO] Track settings: tracker={self.tracker_yaml} conf={self.conf} iou={self.iou} imgsz={self.imgsz}")
        print(f"[INFO] Input: {'RTSP(ffmpeg mjpeg pipe)' if self.use_ffmpeg_rtsp else 'VideoCapture(file)'}")
        print(f"[INFO] Saving output video: {self.output_path}")
        print(f"[INFO] Saving crossings CSV: {self.csv_path}")
        print(f"[INFO] Snapshots directory: {self.snap_dir}")

    def _create_ffmpeg_process(self, url: str):
        vf_parts = []
        if self.rtsp_scale_w is not None:
            vf_parts.append(f"scale={int(self.rtsp_scale_w)}:-1")
        vf = ",".join(vf_parts) if vf_parts else None

        cmd = ["ffmpeg", "-rtsp_transport", self.rtsp_transport, "-i", url, "-an"]
        if vf is not None:
            cmd += ["-vf", vf]

        cmd += ["-r", str(int(round(self.rtsp_fps))),
                "-f", "image2pipe", "-vcodec", "mjpeg", "-"]

        return subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            bufsize=10**8
        )

    def _read_jpeg_from_pipe(self, pipe):
        data = b""
        while True:
            b = pipe.read(1)
            if not b:
                return None
            data += b
            if data.endswith(b"\xff\xd9"):
                return data

    def _read_rtsp_frame(self):
        if self.ffmpeg_proc is None or self.ffmpeg_proc.stdout is None:
            return None
        jpeg_bytes = self._read_jpeg_from_pipe(self.ffmpeg_proc.stdout)
        if jpeg_bytes is None:
            return None
        frame = cv2.imdecode(np.frombuffer(jpeg_bytes, dtype=np.uint8), cv2.IMREAD_COLOR)
        return frame

    def _resolve_car_class_id(self, car_class_id, car_class_name):
        if car_class_id is not None:
            return int(car_class_id)

        if isinstance(self.names, dict):
            name_to_id = {str(v): int(k) for k, v in self.names.items()}
        else:
            name_to_id = {str(v): i for i, v in enumerate(self.names)}

        if car_class_name in name_to_id:
            return int(name_to_id[car_class_name])

        print(f"[WARN] Class name '{car_class_name}' not found in model.names. Falling back to id=0.")
        return 0

    def lane_id_from_y(self, cy):
        if cy < self.lane_y1:
            return 1
        elif cy < self.lane_y2:
            return 2
        else:
            return 3

    def lane_id_from_point(self, cx, cy):
        if self.lane_assigner is not None:
            return self.lane_assigner.lane_id_from_point(cx, cy)
        return self.lane_id_from_y(cy)

    def draw_overlays(self, im0):
        cv2.line(im0, (self.x_cross, 0), (self.x_cross, self.h), (0, 255, 255), 2)
        cv2.putText(im0, f"x={self.x_cross}px", (self.x_cross + 10, 30),
                    cv2.FONT_HERSHEY_SIMPLEX, 0.8, (0, 255, 255), 2)

    def save_snapshot(self, frame_bgr, bbox_xyxy, car_id, t_sec):
        x1, y1, x2, y2 = [int(v) for v in bbox_xyxy]
        x1 = max(0, x1); y1 = max(0, y1)
        x2 = min(self.w - 1, x2); y2 = min(self.h - 1, y2)
        if x2 <= x1 or y2 <= y1:
            return ""
        crop = frame_bgr[y1:y2, x1:x2].copy()
        snap_path = os.path.join(self.snap_dir, f"car_{car_id}_t{t_sec:.3f}.jpg")
        cv2.imwrite(snap_path, crop)
        return snap_path

    def _track_call(self, im0):
        kwargs = dict(persist=True, verbose=False)
        if self.tracker_yaml is not None:
            kwargs["tracker"] = self.tracker_yaml
        if self.conf is not None:
            kwargs["conf"] = float(self.conf)
        if self.iou is not None:
            kwargs["iou"] = float(self.iou)
        if self.imgsz is not None:
            kwargs["imgsz"] = self.imgsz
        return self.model.track(im0, **kwargs)

    def run(self):
        dt = 1.0 / self.fps

        while True:
            if self._first_frame is not None:
                im0 = self._first_frame
                self._first_frame = None
            else:
                if self.use_ffmpeg_rtsp:
                    im0 = self._read_rtsp_frame()
                    if im0 is None:
                        try:
                            if self.ffmpeg_proc is not None:
                                self.ffmpeg_proc.terminate()
                        except Exception:
                            pass
                        self.ffmpeg_proc = self._create_ffmpeg_process(self.source)
                        continue
                else:
                    ok, im0 = self.cap.read()
                    if not ok:
                        print("[INFO] End of video or failed to read frame.")
                        break

            if self._start_unix is None:
                self._start_unix = time.time()
                self._start_monotonic = time.monotonic()

            # actual timestamp for this frame (wall clock, stable)
            frame_unix = self._start_unix + (time.monotonic() - self._start_monotonic)

            t_sec = self._frame_idx / self.fps
            self._frame_idx += 1

            results = self._track_call(im0)
            self.draw_overlays(im0)

            if self.lane_assigner is not None and self.draw_lane_debug:
                self.lane_assigner.draw_debug(im0, thickness=2)

            seen_this_frame = set()
            dets = []

            if results and len(results) > 0:
                r = results[0]
                if r.boxes is not None and r.boxes.id is not None:
                    boxes = r.boxes.xyxy.cpu().numpy()
                    ids = r.boxes.id.cpu().numpy().astype(int)
                    clss = r.boxes.cls.cpu().numpy().astype(int)

                    for box, tid, cls in zip(boxes, ids, clss):
                        if cls != self.car_class_id:
                            continue
                        x1, y1, x2, y2 = [float(v) for v in box]
                        cx = 0.5 * (x1 + x2)
                        cy = 0.5 * (y1 + y2)
                        dets.append((tid, (x1, y1, x2, y2), (cx, cy)))
                        seen_this_frame.add(tid)

            # --- update KF + draw ---
            for tid, bbox, (cx, cy) in dets:
                if tid not in self.kf_by_id:
                    self.kf_by_id[tid] = KF1DConstantVel(
                        dt=dt,
                        meas_noise_std=self.kf_meas_std,
                        accel_noise_std=self.kf_acc_std,
                        init_pos_std=60.0,
                        init_vel_std=80.0,
                    )

                kf = self.kf_by_id[tid]
                front_x = float(bbox[2])  # x2 (left->right front)
                kf.update(front_x)

                self.last_bbox_by_id[tid] = bbox
                self.last_center_by_id[tid] = (cx, cy)
                self.missed_by_id[tid] = 0

                vx = kf.vel()
                speed_mph = abs(vx) * self.PXPS_TO_MPH if vx is not None else 0.0

                col = colors(self.car_class_id, True)
                x1, y1, x2, y2 = map(int, bbox)

                cv2.rectangle(im0, (x1, y1), (x2, y2), col, 2)

                lane_id = self.lane_id_from_point(cx, cy)
                label = f"car:{tid}  lane:{lane_id}  {speed_mph:.1f} mph"
                (tw, th), _ = cv2.getTextSize(label, cv2.FONT_HERSHEY_SIMPLEX, 0.7, 2)
                y_text_top = max(0, y1 - th - 8)
                cv2.rectangle(im0, (x1, y_text_top), (x1 + tw + 8, y1), col, -1)
                cv2.putText(im0, label, (x1 + 4, max(th + 2, y1 - 5)),
                            cv2.FONT_HERSHEY_SIMPLEX, 0.7, (255, 255, 255), 2)

                track = self.track_history[tid]
                track.append((float(cx), float(cy)))
                if len(track) > self.trail_len:
                    track.pop(0)

                pts = np.array(track, dtype=np.int32).reshape((-1, 1, 2))
                cv2.polylines(im0, [pts], isClosed=False, color=col, thickness=self.polyline_thickness)
                cv2.circle(im0, (int(cx), int(cy)), 4, col, -1)

            # --- missed bookkeeping ---
            for tid in list(self.kf_by_id.keys()):
                if tid not in seen_this_frame:
                    self.missed_by_id[tid] += 1

            # --- cleanup stale tracks ---
            for tid in list(self.kf_by_id.keys()):
                if self.missed_by_id[tid] > self.max_missed:
                    self.kf_by_id.pop(tid, None)
                    self.last_bbox_by_id.pop(tid, None)
                    self.last_center_by_id.pop(tid, None)
                    self.prev_front_by_id.pop(tid, None)
                    self.missed_by_id.pop(tid, None)
                    self.track_history.pop(tid, None)
                    self.crossed_ids.discard(tid)

            # --- crossing detection using car FRONT (x2) ---
            for tid in seen_this_frame:
                if tid in self.crossed_ids:
                    if tid in self.last_bbox_by_id:
                        self.prev_front_by_id[tid] = (float(self.last_bbox_by_id[tid][2]), t_sec)
                    continue

                bbox = self.last_bbox_by_id.get(tid)
                kf = self.kf_by_id.get(tid)
                center = self.last_center_by_id.get(tid)
                if bbox is None or kf is None or center is None:
                    continue

                cx, cy = center
                front = float(bbox[2])  # x2

                prev = self.prev_front_by_id.get(tid)
                if prev is not None:
                    prev_front, prev_t = prev
                    if prev_front < self.x_cross <= front:
                        alpha = (self.x_cross - prev_front) / max(1e-6, (front - prev_front))
                        t_cross = prev_t + alpha * (t_sec - prev_t)

                        # actual timestamp for the crossing moment (interpolated within current frame interval)
                        t_cross_unix = frame_unix - (t_sec - t_cross)

                        vx = kf.vel()
                        speed_mph = abs(vx) * self.PXPS_TO_MPH if vx is not None else 0.0
                        lane_id = self.lane_id_from_point(cx, cy)

                        snap_path = self.save_snapshot(im0, bbox, tid, t_cross)

                        self.csv_writer.writerow([
                            int(tid),
                            int(lane_id),
                            f"{t_cross_unix:.6f}",
                            f"{float(self.x_cross):.2f}",
                            f"{vx:.3f}" if vx is not None else "",
                            f"{speed_mph:.3f}",
                            snap_path,
                        ])
                        self.csv_file.flush()

                        print(f"[EVENT] car_id={tid} lane={lane_id} t_unix={t_cross_unix:.3f} vx={vx:.2f}px/s mph={speed_mph:.1f}")
                        self.crossed_ids.add(tid)

                self.prev_front_by_id[tid] = (front, t_sec)

            cv2.putText(im0, f"cars={len(dets)}  kfs={len(self.kf_by_id)}  out={Path(self.save_dir).name}",
                        (20, 50), cv2.FONT_HERSHEY_SIMPLEX, 0.9, (0, 255, 255), 2)

            self.writer.write(im0)
            cv2.imshow(self.window_name, im0)

            key = cv2.waitKey(1) & 0xFF
            if key == ord("q"):
                break

        try:
            if not self.use_ffmpeg_rtsp:
                self.cap.release()
        except Exception:
            pass

        try:
            if self.ffmpeg_proc is not None:
                self.ffmpeg_proc.terminate()
        except Exception:
            pass

        self.writer.release()
        self.csv_file.close()
        cv2.destroyAllWindows()
        print(f"[INFO] Done. Saved video: {self.output_path}")
        print(f"[INFO] CSV: {self.csv_path}")


if __name__ == "__main__":
    tracker = CarCrossingTrackerStable(
        model="/Users/anuskadas/PycharmProjects/Jan25/Camera_Final/train/weights/best.pt",
        source="rtsp://admin:1ubuntu9@@localhost:8554",
        save_dir="yolo26_runs/exp",
        output_name="car-tracking.mp4",

        tracker_yaml=None,
        conf=None,
        iou=None,
        imgsz=None,

        x_cross_ratio=0.75,
        fov_meters=30.0,
        max_missed=30,

        lanes_json="lanes.json",
        draw_lane_debug=True,

        rtsp_transport="tcp",
        rtsp_scale_w=960,
        ###rtsp_scale_w=None,
        rtsp_fps=20.0,
    )
    tracker.run()