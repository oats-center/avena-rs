import cv2
import json
import numpy as np

NUM_BOUNDARIES = 4          # 4 boundaries -> 3 lanes
DEGREE = 3              
N_SAMPLES = 80              # samples for lane boundaries (x=f(y))
ROI_SAMPLES = 140           # samples for ROI curves (y=f(x))
WINDOW = "Lane Annotator"

def sort_by_y(points):
    pts = np.array(points, dtype=np.float32)
    idx = np.argsort(pts[:, 1])
    return pts[idx]

def poly_smooth_boundary_x_of_y(points, y_min, y_max, degree=3, n_samples=60):
    """
    Fit x = f(y) polynomial to clicked points, then sample.
    Returns sampled boundary points as int32 Nx2 (in increasing y).
    """
    if len(points) < max(4, degree + 1):
        return None

    pts = sort_by_y(points)
    ys = pts[:, 1]
    xs = pts[:, 0]

    deg = min(degree, len(points) - 1)
    coef = np.polyfit(ys, xs, deg)

    y_vals = np.linspace(y_min, y_max, n_samples)
    x_vals = np.polyval(coef, y_vals)

    boundary = np.stack([x_vals, y_vals], axis=1)
    boundary = np.round(boundary).astype(np.int32)
    return boundary

def poly_smooth_curve_y_of_x(points, x_min, x_max, degree=3, n_samples=120):
    """
    Fit y = f(x) polynomial to clicked points, then sample along x.
    Returns sampled curve points as int32 Nx2 (in increasing x).
    """
    if len(points) < max(4, degree + 1):
        return None

    pts = np.array(points, dtype=np.float32)
    pts = pts[np.argsort(pts[:, 0])]  # sort by x

    xs = pts[:, 0]
    ys = pts[:, 1]

    deg = min(degree, len(points) - 1)
    coef = np.polyfit(xs, ys, deg)

    x_vals = np.linspace(x_min, x_max, n_samples)
    y_vals = np.polyval(coef, x_vals)

    curve = np.stack([x_vals, y_vals], axis=1)
    return np.round(curve).astype(np.int32)

def make_lane_polygon(boundary_left, boundary_right):
    """
    Polygon between two boundaries sampled in increasing y.
    """
    poly = np.vstack([boundary_left, boundary_right[::-1]])
    return poly.astype(np.int32)

def draw_polyline(img, pts, color, thickness=2, closed=False):
    if pts is None or len(pts) < 2:
        return
    cv2.polylines(img, [pts], isClosed=closed, color=color, thickness=thickness)

def draw_points(img, pts, color):
    for (x, y) in pts:
        cv2.circle(img, (int(x), int(y)), 4, color, -1)

def polygon_mask(H, W, poly_pts):
    mask = np.zeros((H, W), dtype=np.uint8)
    if poly_pts is None or len(poly_pts) < 3:
        return mask
    cv2.fillPoly(mask, [np.array(poly_pts, dtype=np.int32)], 255)
    return mask

# ----------------------------
# Mouse callback state
# ----------------------------
clicked = [[] for _ in range(NUM_BOUNDARIES)]
active_b = 0

# ROI curved band: top curve and bottom curve
roi_top = []
roi_bot = []
roi_mode = 0  # 0=off, 1=top, 2=bottom

# If True, boundary clicks outside ROI are ignored (once ROI is valid)
ENFORCE_ROI_FOR_BOUNDARIES = True

def on_mouse(event, x, y, flags, param):
    global clicked, active_b, roi_mode, roi_top, roi_bot

    if event != cv2.EVENT_LBUTTONDOWN:
        return

    if roi_mode == 1:
        roi_top.append((x, y))
        return
    if roi_mode == 2:
        roi_bot.append((x, y))
        return

    # optional ROI enforcement
    if ENFORCE_ROI_FOR_BOUNDARIES:
        roi_mask = param.get("roi_mask", None)
        if roi_mask is not None and roi_mask.shape[0] > y and roi_mask.shape[1] > x:
            if roi_mask[y, x] == 0:
                return

    clicked[active_b].append((x, y))

def main(image_path, out_json="lanes.json"):
    global active_b, clicked, roi_top, roi_bot, roi_mode

    img0 = cv2.imread(image_path)
    if img0 is None:
        raise FileNotFoundError(f"Could not read image: {image_path}")

    H, W = img0.shape[:2]

    # For lane boundaries: restrict fitting to a vertical band (tweak)
    y_min = int(H * 0.40)
    y_max = int(H * 0.90)

    cv2.namedWindow(WINDOW, cv2.WINDOW_NORMAL)

    print("\nControls:")
    print("  1/2/3/4 : select lane boundary (B0..B3)")
    print("  t       : ROI TOP mode (click points along upper road edge)")
    print("  b       : ROI BOTTOM mode (click points along lower road edge)")
    print("  n       : ROI mode off (back to boundaries)")
    print("  u       : undo last point (ROI mode: undo ROI point; boundary mode: undo boundary point)")
    print("  c       : clear active boundary points")
    print("  C       : clear ROI points (top+bottom)")
    print("  s       : save JSON (ROI + boundaries + lane polygons)")
    print("  q/ESC   : quit\n")

    colors = [(0, 0, 255), (0, 255, 0), (255, 0, 0), (255, 255, 0)]  # B0..B3
    lane_fill_colors = [(60, 60, 255), (60, 255, 60), (255, 60, 60)]

    # dummy roi mask init
    roi_mask = None

    while True:
        vis = img0.copy()

        # --- Build curved ROI band if possible ---
        x_min = 0
        x_max = W - 1

        top_curve = poly_smooth_curve_y_of_x(
            roi_top, x_min, x_max, degree=DEGREE, n_samples=ROI_SAMPLES
        )
        bot_curve = poly_smooth_curve_y_of_x(
            roi_bot, x_min, x_max, degree=DEGREE, n_samples=ROI_SAMPLES
        )

        roi_poly = None
        if top_curve is not None and bot_curve is not None:
            roi_poly = np.vstack([top_curve, bot_curve[::-1]]).astype(np.int32)
            # visualize ROI
            overlay = vis.copy()
            cv2.fillPoly(overlay, [roi_poly], (0, 255, 255))
            vis = cv2.addWeighted(overlay, 0.12, vis, 0.88, 0)
            cv2.polylines(vis, [roi_poly], True, (0, 255, 255), 2)
            roi_mask = polygon_mask(H, W, roi_poly.tolist())
        else:
            roi_mask = None
            # show in-progress ROI clicks
            if len(roi_top) > 0:
                draw_points(vis, roi_top, (0, 255, 255))
                if len(roi_top) >= 2:
                    cv2.polylines(vis, [np.array(roi_top, np.int32)], False, (0, 255, 255), 2)
            if len(roi_bot) > 0:
                draw_points(vis, roi_bot, (0, 200, 255))
                if len(roi_bot) >= 2:
                    cv2.polylines(vis, [np.array(roi_bot, np.int32)], False, (0, 200, 255), 2)

        # update mouse callback param so clicks can be filtered by ROI mask
        cv2.setMouseCallback(WINDOW, on_mouse, param={"roi_mask": roi_mask})

        # status text
        mode_txt = "BOUNDARIES"
        if roi_mode == 1:
            mode_txt = "ROI TOP"
        elif roi_mode == 2:
            mode_txt = "ROI BOTTOM"

        cv2.putText(
            vis,
            f"Mode: {mode_txt} | Active boundary: B{active_b} (press 1-4) | ROI: t/b/n",
            (20, 40),
            cv2.FONT_HERSHEY_SIMPLEX,
            0.8,
            (0, 255, 255),
            2,
        )

        # show lane-fit band reference
        cv2.rectangle(vis, (0, y_min), (W - 1, y_max), (0, 180, 180), 2)
        cv2.putText(
            vis,
            f"Lane fit band y:[{y_min},{y_max}]",
            (20, 70),
            cv2.FONT_HERSHEY_SIMPLEX,
            0.7,
            (0, 180, 180),
            2,
        )

        # --- Draw clicked points + smoothed lane boundaries ---
        smoothed = []
        for i in range(NUM_BOUNDARIES):
            pts = clicked[i]
            draw_points(vis, pts, colors[i])

            b = poly_smooth_boundary_x_of_y(
                pts, y_min, y_max, degree=DEGREE, n_samples=N_SAMPLES
            )
            smoothed.append(b)

            if b is not None:
                draw_polyline(vis, b, colors[i], thickness=3, closed=False)

            cv2.putText(
                vis,
                f"B{i}: {len(pts)} pts",
                (20, 110 + i * 28),
                cv2.FONT_HERSHEY_SIMPLEX,
                0.7,
                colors[i],
                2,
            )

        # --- If all 4 boundaries exist, preview 3 lane polygons ---
        if all(b is not None for b in smoothed):
            lane_polys = [
                make_lane_polygon(smoothed[0], smoothed[1]),
                make_lane_polygon(smoothed[1], smoothed[2]),
                make_lane_polygon(smoothed[2], smoothed[3]),
            ]

            overlay = vis.copy()
            for i, poly in enumerate(lane_polys):
                cv2.fillPoly(overlay, [poly], lane_fill_colors[i])
                cv2.polylines(vis, [poly], True, (255, 255, 255), 2)
                mid = poly[len(poly) // 2]
                cv2.putText(
                    vis,
                    f"Lane {i+1}",
                    (int(mid[0]), int(mid[1])),
                    cv2.FONT_HERSHEY_SIMPLEX,
                    0.85,
                    (255, 255, 255),
                    2,
                )

            vis = cv2.addWeighted(overlay, 0.18, vis, 0.82, 0)

        cv2.imshow(WINDOW, vis)
        key = cv2.waitKey(30) & 0xFF

        if key in [27, ord("q")]:  # ESC or q
            break

        # boundary selection
        elif key in [ord("1"), ord("2"), ord("3"), ord("4")]:
            active_b = int(chr(key)) - 1

        # ROI mode
        elif key == ord("t"):
            roi_mode = 1
            print("ROI TOP mode: click points along upper road edge (press n to exit ROI mode).")
        elif key == ord("b"):
            roi_mode = 2
            print("ROI BOTTOM mode: click points along lower road edge (press n to exit ROI mode).")
        elif key == ord("n"):
            roi_mode = 0
            print("ROI mode off: back to boundaries.")

        # undo / clear
        elif key == ord("u"):
            if roi_mode == 1:
                if roi_top:
                    roi_top.pop()
            elif roi_mode == 2:
                if roi_bot:
                    roi_bot.pop()
            else:
                if clicked[active_b]:
                    clicked[active_b].pop()

        elif key == ord("c"):
            if roi_mode != 0:
                print("Use 'C' to clear ROI. 'c' clears active boundary.")
            else:
                clicked[active_b] = []

        elif key == ord("C"):
            roi_top = []
            roi_bot = []
            print("Cleared ROI (top + bottom).")

        # save
        elif key == ord("s"):
            # Validate ROI
            top_curve_s = poly_smooth_curve_y_of_x(
                roi_top, 0, W - 1, degree=DEGREE, n_samples=ROI_SAMPLES
            )
            bot_curve_s = poly_smooth_curve_y_of_x(
                roi_bot, 0, W - 1, degree=DEGREE, n_samples=ROI_SAMPLES
            )

            if top_curve_s is None or bot_curve_s is None:
                print(f"Cannot save: ROI needs >= {max(4, DEGREE+1)} points on BOTH top and bottom.")
                continue

            roi_poly_s = np.vstack([top_curve_s, bot_curve_s[::-1]]).astype(np.int32)

            # Validate boundaries
            smoothed_s = []
            for i in range(NUM_BOUNDARIES):
                b = poly_smooth_boundary_x_of_y(
                    clicked[i], y_min, y_max, degree=DEGREE, n_samples=N_SAMPLES
                )
                if b is None:
                    print(f"Cannot save: boundary B{i} needs at least {max(4, DEGREE+1)} points.")
                    smoothed_s = None
                    break
                smoothed_s.append(b)

            if smoothed_s is None:
                continue

            lane_polys_s = [
                make_lane_polygon(smoothed_s[0], smoothed_s[1]),
                make_lane_polygon(smoothed_s[1], smoothed_s[2]),
                make_lane_polygon(smoothed_s[2], smoothed_s[3]),
            ]

            data = {
                "image_path": image_path,
                "image_size": {"width": W, "height": H},
                "lane_fit_band": {"y_min": y_min, "y_max": y_max},
                "roi": {
                    "roi_top_clicked": roi_top,
                    "roi_bottom_clicked": roi_bot,
                    "roi_top_curve": top_curve_s.tolist(),
                    "roi_bottom_curve": bot_curve_s.tolist(),
                    "roi_polygon": roi_poly_s.tolist(),
                },
                "boundaries": {f"B{i}": smoothed_s[i].tolist() for i in range(NUM_BOUNDARIES)},
                "lanes": {
                    "lane1": lane_polys_s[0].tolist(),
                    "lane2": lane_polys_s[1].tolist(),
                    "lane3": lane_polys_s[2].tolist(),
                },
            }

            with open(out_json, "w") as f:
                json.dump(data, f, indent=2)

            print(f"Saved: {out_json}")

    cv2.destroyAllWindows()

if __name__ == "__main__":
    main("/Users/anuskadas/Desktop/1.png", out_json="lanes.json")
